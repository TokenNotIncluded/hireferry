//! Strict HumanGrant 0.1 HTTP signature profile.
//!
//! Adapted from LIghtJUNction/humangrant src/crypto.rs, commit
//! c4a730480d2aae93a636889a144a025fe98becb8, under the MIT license.
//! See THIRD_PARTY_NOTICES for attribution. This is not a general RFC 9421 parser.

use super::{Error, Result};
use axum::http::{HeaderMap, Method, Uri};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use ed25519_dalek::{Signature, VerifyingKey};
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};

const COMPONENTS: &str = "(\"@method\" \"@target-uri\" \"content-digest\" \"content-type\" \"x-hg-key\" \"authorization\")";

pub(super) fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub(super) fn hash(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(bytes))
}

pub(super) struct Proof {
    pub actor_id: String,
    pub public_key: String,
    pub nonce: String,
    pub expires_at: i64,
    pub credential: String,
}

fn bad() -> Error {
    Error::new(
        401,
        "invalid_signature",
        "A valid, unexpired HumanGrant signature is required.",
    )
}

fn one<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next().ok_or_else(bad)?;
    if values.next().is_some() {
        return Err(bad());
    }
    value.to_str().map_err(|_| bad())
}

pub(super) fn verify(
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &[u8],
    origin: &str,
    now: i64,
) -> Result<Proof> {
    let path = uri.path();
    if uri.query().is_some()
        || path.len() > 512
        || path.contains(['%', '\\'])
        || path.contains("//")
        || (path.len() > 1 && path.ends_with('/'))
        || (method == Method::GET && !body.is_empty())
    {
        return Err(Error::invalid());
    }
    if body.len() > super::MAX_BODY {
        return Err(Error::body_too_large());
    }
    if one(headers, "content-type")? != "application/json" {
        return Err(bad());
    }
    let encoded = one(headers, "x-hg-key")?;
    let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| bad())?;
    if URL_SAFE_NO_PAD.encode(&bytes) != encoded {
        return Err(bad());
    }
    let raw: [u8; 32] = bytes.try_into().map_err(|_| bad())?;
    let public = VerifyingKey::from_bytes(&raw).map_err(|_| bad())?;
    let actor = format!("ed25519:{}", hash(&raw));
    let digest = format!("sha-256=:{}:", STANDARD.encode(Sha256::digest(body)));
    if one(headers, "content-digest")? != digest {
        return Err(bad());
    }

    let input = one(headers, "signature-input")?;
    let prefix = format!("hg={COMPONENTS};created=");
    let rest = input.strip_prefix(&prefix).ok_or_else(bad)?;
    let (created, rest) = rest.split_once(";expires=").ok_or_else(bad)?;
    let (expires, rest) = rest.split_once(";nonce=\"").ok_or_else(bad)?;
    let (nonce, rest) = rest.split_once("\";keyid=\"").ok_or_else(bad)?;
    let (supplied_id, end) = rest
        .split_once("\";alg=\"ed25519\";tag=\"humangrant-0.1\"")
        .ok_or_else(bad)?;
    let created: i64 = created.parse().map_err(|_| bad())?;
    let expires: i64 = expires.parse().map_err(|_| bad())?;
    let lifetime = expires.checked_sub(created).ok_or_else(bad)?;
    if !end.is_empty()
        || supplied_id != actor
        || created < 0
        || created > now.saturating_add(30)
        || expires <= now
        || !(1..=60).contains(&lifetime)
    {
        return Err(bad());
    }
    let nonce_bytes = URL_SAFE_NO_PAD.decode(nonce).map_err(|_| bad())?;
    if nonce_bytes.len() != 24 || URL_SAFE_NO_PAD.encode(&nonce_bytes) != nonce {
        return Err(bad());
    }
    let params = format!(
        "{COMPONENTS};created={created};expires={expires};nonce=\"{nonce}\";keyid=\"{actor}\";alg=\"ed25519\";tag=\"humangrant-0.1\""
    );
    if input != format!("hg={params}") {
        return Err(bad());
    }
    let authorization = one(headers, "authorization")?;
    let token = authorization.strip_prefix("HumanGrant ").ok_or_else(bad)?;
    if token != "anonymous" {
        let token_bytes = URL_SAFE_NO_PAD.decode(token).map_err(|_| bad())?;
        if token_bytes.len() != 32 || URL_SAFE_NO_PAD.encode(&token_bytes) != token {
            return Err(bad());
        }
    }
    let signature = one(headers, "signature")?
        .strip_prefix("hg=:")
        .and_then(|s| s.strip_suffix(':'))
        .ok_or_else(bad)?;
    let signature_bytes = STANDARD.decode(signature).map_err(|_| bad())?;
    if STANDARD.encode(&signature_bytes) != signature {
        return Err(bad());
    }
    let signature = Signature::from_slice(&signature_bytes).map_err(|_| bad())?;
    let target = format!("{}{path}", origin.trim_end_matches('/'));
    let base = format!(
        "\"@method\": {method}\n\"@target-uri\": {target}\n\"content-digest\": {digest}\n\"content-type\": application/json\n\"x-hg-key\": {encoded}\n\"authorization\": {authorization}\n\"@signature-params\": {params}"
    );
    public
        .verify_strict(base.as_bytes(), &signature)
        .map_err(|_| bad())?;
    Ok(Proof {
        actor_id: actor,
        public_key: encoded.into(),
        nonce: nonce.into(),
        expires_at: expires,
        credential: token.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_signature_vector_and_mutations() {
        // Public vector only, from upstream tests/fixtures/signature.json.
        let body = br#"{"client_id":"example-cli","target_service":"notes","purpose":"Read a note","requested_permissions":[{"action":"notes.read","resource":"alice-note"}],"expected_human_id":"alice"}"#;
        let mut headers = HeaderMap::new();
        for (name, value) in [
            ("content-type", "application/json"),
            (
                "content-digest",
                "sha-256=:jN6odj5Tli9ZfoLdtLxtE/sUNtMKAKM2V7xq8/qri5Y=:",
            ),
            ("x-hg-key", "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo"),
            ("authorization", "HumanGrant anonymous"),
            (
                "signature-input",
                "hg=(\"@method\" \"@target-uri\" \"content-digest\" \"content-type\" \"x-hg-key\" \"authorization\");created=1700000000;expires=1700000060;nonce=\"AAECAwQFBgcICQoLDA0ODxAREhMUFRYX\";keyid=\"ed25519:If4x36FUomFia_hUBG_SJxt77UtqvkWqWId-9H-XIbk\";alg=\"ed25519\";tag=\"humangrant-0.1\"",
            ),
            (
                "signature",
                "hg=:jmjuzgV58zQY0A8N+T9sss24Jwa8Kah5bvs2VSdWA43TScypSfoPkzlERC6XLBOvx8+D+ZqQ0WOVKwS2/Gd3AA==:",
            ),
        ] {
            headers.insert(name, value.parse().unwrap());
        }
        let uri: Uri = "/v1/requests".parse().unwrap();
        assert!(
            verify(
                &Method::POST,
                &uri,
                &headers,
                body,
                "https://auth.example.test",
                1_700_000_000
            )
            .is_ok()
        );
        assert!(
            verify(
                &Method::POST,
                &uri,
                &headers,
                b"{}",
                "https://auth.example.test",
                1_700_000_000
            )
            .is_err()
        );
        assert!(
            verify(
                &Method::POST,
                &uri,
                &headers,
                body,
                "https://auth.example.test",
                1_700_000_060
            )
            .is_err()
        );
        assert!(
            verify(
                &Method::POST,
                &"/v1/requests?x=1".parse().unwrap(),
                &headers,
                body,
                "https://auth.example.test",
                1_700_000_000
            )
            .is_err()
        );
        headers.append("x-hg-key", headers["x-hg-key"].clone());
        assert!(
            verify(
                &Method::POST,
                &uri,
                &headers,
                body,
                "https://auth.example.test",
                1_700_000_000
            )
            .is_err()
        );
    }
}
