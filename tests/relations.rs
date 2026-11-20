//! The graph's truth is independent of list windows. All fixtures are synthetic.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use hireferry::{
    AppState,
    access::Principal,
    auth,
    config::Config,
    relations::{self, Filters, Focus, Grouping, LeftGrouping, Mode, Query, RightGrouping, Side},
};
use http_body_util::BodyExt;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{collections::HashSet, time::Instant};
use tower::ServiceExt;

fn state() -> AppState {
    let mut cfg = Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    };
    cfg.webhooks.enabled = false;
    AppState::new(cfg).unwrap()
}
fn admin() -> Principal {
    Principal {
        user_id: None,
        admin: true,
        via_ai: false,
        actor: "admin".into(),
    }
}
fn user(db: &Connection, id: &str, role: &str, name: &str) {
    db.execute("INSERT INTO users(id,email,name,role,resume_md,created_at,updated_at) VALUES(?1,?2,?3,?4,'PRIVATE_RESUME_SENTINEL',1000,1000)",params![id,format!("{id}@example.invalid"),name,role]).unwrap();
}
fn job(db: &Connection, id: &str, owner: &str, status: &str) {
    db.execute("INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,headcount,status,created_at,updated_at) VALUES(?1,?2,?1,'PRIVATE_JOB_SENTINEL',500000,1,?3,1000,1000)",params![id,owner,status]).unwrap();
}
fn mandate(db: &Connection, id: &str, industry: &str, roles: &str) {
    db.execute("INSERT INTO mandates(user_id,target_industry,target_roles,location,preferences,created_at,updated_at) VALUES(?1,?2,?3,'Remote','PRIVATE_PREFERENCES_SENTINEL',1000,1000)",params![id,industry,roles]).unwrap();
}
fn matched(db: &Connection, id: &str, seeker: &str, job: &str, status: &str) {
    db.execute("INSERT INTO matches(id,seeker_id,job_id,status,note,created_at,updated_at) VALUES(?1,?2,?3,?4,'PRIVATE_NOTE_SENTINEL',1000,1000)",params![id,seeker,job,status]).unwrap();
}
fn fixture() -> AppState {
    let s = state();
    {
        let db = s.conn().unwrap();
        user(&db, "owner", "employer", "Employer");
        user(&db, "a", "seeker", "Candidate A 100%_");
        user(&db, "b", "seeker", "Candidate B");
        user(&db, "c", "seeker", "Candidate C");
        user(&db, "unset", "unassigned", "Undecided");
        mandate(&db, "a", "Software", "Rust, Go");
        mandate(&db, "b", "Software", "Rust, Go");
        mandate(&db, "c", "Finance", "Research");
        job(&db, "job-a", "owner", "published");
        job(&db, "job-b", "owner", "closed");
        job(&db, "job-c", "owner", "draft");
        matched(&db, "m-a", "a", "job-a", "introduced");
        matched(&db, "m-b", "b", "job-a", "interview");
        matched(&db, "m-c", "a", "job-b", "declined");
        matched(&db, "m-d", "c", "job-c", "hired");
        // Legacy malformed data must never turn the employer/job ownership
        // relationship into a candidate match in this view.
        matched(&db, "legacy-owner", "owner", "job-a", "introduced");
    }
    s
}
fn query(s: &AppState, q: &Query) -> Value {
    relations::query(s, &admin(), q).unwrap()
}
fn nodes(side: Side) -> Query {
    Query {
        mode: Mode::Nodes,
        side: Some(side),
        ..Default::default()
    }
}
fn items(v: &Value) -> &Vec<Value> {
    v["items"].as_array().unwrap()
}

#[test]
fn overview_is_complete_without_any_directory_page_and_counts_are_unique() {
    let s = fixture();
    let v = query(&s, &Query::default());
    assert_eq!(
        v["summary"],
        json!({"users":5,"seekers":3,"jobs":3,"edges":4,"matched_users":3,"matched_jobs":3,"possible_pairs":9,"pair_scope":"filtered_directory","state_counts":{"introduced":1,"interview":1,"hired":1,"declined":1,"unknown":0}})
    );
    for side in ["left", "right"] {
        assert_eq!(
            v["groups"][side]
                .as_array()
                .unwrap()
                .iter()
                .map(|g| g["edges"].as_i64().unwrap())
                .sum::<i64>(),
            4
        );
    }
    assert_eq!(
        v["bundles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["count"].as_i64().unwrap())
            .sum::<i64>(),
        4
    );
    let first = query(
        &s,
        &Query {
            limit: Some(1),
            ..nodes(Side::Left)
        },
    );
    assert_eq!(items(&first).len(), 1);
    assert_eq!(first["total"], 5);
    assert_eq!(first["remaining"], 4);
    assert_eq!(query(&s, &Query::default())["summary"], v["summary"]);
    let employer = query(
        &s,
        &Query {
            filters: Filters {
                role: Some("employer".into()),
                ..Default::default()
            },
            ..nodes(Side::Left)
        },
    );
    assert_eq!(employer["items"][0]["id"], "owner");
    assert_eq!(employer["items"][0]["relation_count"], 0);
    for payload in [v, first, employer] {
        let serialized = payload.to_string();
        assert!(!serialized.contains("@example.invalid"));
        assert!(!serialized.contains("PRIVATE_"));
        assert!(!serialized.contains("resume_md"));
    }
}

#[test]
fn filters_apply_in_sql_and_explicit_empty_sets_do_not_widen_scope() {
    let s = fixture();
    let filtered = Query {
        filters: Filters {
            job_status: Some("published".into()),
            match_status: Some("interview".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let v = query(&s, &filtered);
    assert_eq!(v["summary"]["users"], 5);
    assert_eq!(v["summary"]["jobs"], 1);
    assert_eq!(v["summary"]["edges"], 1);
    assert_eq!(v["summary"]["state_counts"]["interview"], 1);
    assert_eq!(v["summary"]["state_counts"]["introduced"], 0);
    let literal = query(
        &s,
        &Query {
            filters: Filters {
                user_q: Some("100%_".into()),
                ..Default::default()
            },
            ..nodes(Side::Left)
        },
    );
    assert_eq!(literal["total"], 1);
    assert_eq!(literal["items"][0]["id"], "a");
    let by_email = query(
        &s,
        &Query {
            filters: Filters {
                user_q: Some("a@example.invalid".into()),
                ..Default::default()
            },
            ..nodes(Side::Left)
        },
    );
    assert_eq!(by_email["total"], 1);
    assert_eq!(by_email["items"][0]["id"], "a");
    assert!(!by_email.to_string().contains("a@example.invalid"));
    let injected = query(
        &s,
        &Query {
            filters: Filters {
                job_q: Some("'); DROP TABLE jobs;--".into()),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert_eq!(injected["summary"]["jobs"], 0);
    for f in [
        Filters {
            user_ids: Some(vec![]),
            ..Default::default()
        },
        Filters {
            job_ids: Some(vec![]),
            ..Default::default()
        },
        Filters {
            left_group_ids: Some(vec![]),
            ..Default::default()
        },
    ] {
        assert_eq!(
            query(
                &s,
                &Query {
                    filters: f,
                    ..Default::default()
                }
            )["summary"]["edges"],
            0
        );
    }
    let empty = state();
    assert_eq!(query(&empty, &Query::default())["summary"]["edges"], 0);
    assert!(
        query(&empty, &Query::default())["groups"]["left"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        relations::query(
            &s,
            &admin(),
            &Query {
                limit: Some(101),
                ..nodes(Side::Left)
            }
        )
        .is_err()
    );
    assert!(serde_json::from_value::<Query>(json!({"filters":{"made_up":true}})).is_err());
    matched(
        &s.conn().unwrap(),
        "unknown-stage",
        "b",
        "job-b",
        "legacy-value",
    );
    let unknown = query(
        &s,
        &Query {
            filters: Filters {
                match_status: Some("unknown".into()),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert_eq!(unknown["summary"]["edges"], 1);
    assert_eq!(unknown["summary"]["state_counts"]["unknown"], 1);
}

#[test]
fn neighbors_are_not_limited_to_visible_nodes_and_page_cursor_is_bound() {
    let s = fixture();
    let q = Query {
        mode: Mode::Neighbors,
        focus: Some(Focus {
            side: Side::Left,
            id: "a".into(),
        }),
        limit: Some(1),
        ..Default::default()
    };
    let first = query(&s, &q);
    assert_eq!(first["total"], 2);
    assert_eq!(first["focus"]["relation_count"], 2);
    assert_eq!(first["remaining"], 1);
    assert_eq!(first["state_counts"]["declined"], 1);
    let cursor = first["next_cursor"].as_str().unwrap().to_owned();
    let second = query(
        &s,
        &Query {
            cursor: Some(cursor.clone()),
            limit: Some(100),
            data_rev: first["data_rev"].as_i64(),
            ..q.clone()
        },
    );
    assert_eq!(second["remaining"], 0);
    assert_eq!(items(&second).len(), 1);
    assert!(second["next_cursor"].is_null());
    assert_ne!(
        first["items"][0]["edge"]["id"],
        second["items"][0]["edge"]["id"]
    );
    assert_eq!(first["query_key"], second["query_key"]);
    for changed in [
        Query {
            filters: Filters {
                match_status: Some("declined".into()),
                ..Default::default()
            },
            ..q.clone()
        },
        Query {
            focus: Some(Focus {
                side: Side::Right,
                id: "job-a".into(),
            }),
            ..q.clone()
        },
    ] {
        let error = relations::query(
            &s,
            &admin(),
            &Query {
                cursor: Some(cursor.clone()),
                ..changed
            },
        )
        .unwrap_err();
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }
    let mut tampered = cursor.into_bytes();
    tampered[0] = if tampered[0] == b'A' { b'B' } else { b'A' };
    assert_eq!(
        relations::query(
            &s,
            &admin(),
            &Query {
                cursor: Some(String::from_utf8(tampered).unwrap()),
                ..q
            }
        )
        .unwrap_err()
        .status,
        StatusCode::BAD_REQUEST
    );
    let by_job = query(
        &s,
        &Query {
            mode: Mode::Neighbors,
            focus: Some(Focus {
                side: Side::Right,
                id: "job-a".into(),
            }),
            ..Default::default()
        },
    );
    assert_eq!(by_job["total"], 2);
    assert!(
        items(&by_job)
            .iter()
            .all(|item| item["node"]["role"] == "seeker")
    );
}

#[test]
fn nodes_page_by_stable_identity_even_when_created_timestamps_are_equal() {
    let s = fixture();
    let mut cursor = None;
    let mut ids = HashSet::new();
    loop {
        let v = query(
            &s,
            &Query {
                cursor,
                limit: Some(2),
                ..nodes(Side::Left)
            },
        );
        for node in items(&v) {
            assert!(ids.insert(node["id"].as_str().unwrap().to_owned()));
        }
        cursor = v["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            assert_eq!(v["remaining"], 0);
            break;
        }
    }
    assert_eq!(ids.len(), 5);
}

#[test]
fn graph_revisions_cover_metadata_group_membership_and_deletions() {
    let s = fixture();
    for mutation in [
        "UPDATE users SET name='Renamed' WHERE id='a'",
        "UPDATE mandates SET target_industry='Changed' WHERE user_id='a'",
        "UPDATE jobs SET title='Changed title' WHERE id='job-a'",
        "UPDATE matches SET status='hired' WHERE id='m-a'",
        "DELETE FROM matches WHERE id='m-a'",
    ] {
        let first = query(
            &s,
            &Query {
                limit: Some(1),
                ..nodes(Side::Left)
            },
        );
        s.conn().unwrap().execute(mutation, []).unwrap();
        let error = relations::query(
            &s,
            &admin(),
            &Query {
                cursor: first["next_cursor"].as_str().map(str::to_owned),
                ..nodes(Side::Left)
            },
        )
        .unwrap_err();
        assert_eq!(error.status, StatusCode::CONFLICT);
        assert!(error.message.contains("resync_required"));
        assert_eq!(
            relations::query(
                &s,
                &admin(),
                &Query {
                    data_rev: first["data_rev"].as_i64(),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .status,
            StatusCode::CONFLICT
        );
    }
    let rev = relations::revision(&s.conn().unwrap()).unwrap();
    relations::migrate(&s.conn().unwrap()).unwrap();
    assert_eq!(relations::revision(&s.conn().unwrap()).unwrap(), rev);
}

#[test]
fn authorization_is_rechecked_on_every_page_and_cursor_is_principal_bound() {
    let s = fixture();
    let ordinary = Principal {
        user_id: Some("a".into()),
        admin: false,
        via_ai: false,
        actor: "a".into(),
    };
    assert_eq!(
        relations::query(&s, &ordinary, &Query::default())
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
    s.conn().unwrap().execute("INSERT INTO staff_members(user_id,role,active,joined_at,updated_at) VALUES('a','manager',1,1000,1000)",[]).unwrap();
    let manager = Principal {
        user_id: Some("a".into()),
        admin: true,
        via_ai: false,
        actor: "staff:a".into(),
    };
    // Prime and reuse the aggregate cache, then revoke authority without a
    // graph-data change. Cache hits must still check current membership.
    relations::query(&s, &manager, &Query::default()).unwrap();
    relations::query(&s, &manager, &Query::default()).unwrap();
    let first = relations::query(
        &s,
        &manager,
        &Query {
            limit: Some(1),
            ..nodes(Side::Left)
        },
    )
    .unwrap();
    let next = Query {
        cursor: first["next_cursor"].as_str().map(str::to_owned),
        ..nodes(Side::Left)
    };
    assert_eq!(
        relations::query(&s, &admin(), &next).unwrap_err().status,
        StatusCode::BAD_REQUEST
    );
    let rev = relations::revision(&s.conn().unwrap()).unwrap();
    s.conn()
        .unwrap()
        .execute("UPDATE staff_members SET active=0 WHERE user_id='a'", [])
        .unwrap();
    assert_eq!(relations::revision(&s.conn().unwrap()).unwrap(), rev);
    assert_eq!(
        relations::query(&s, &manager, &next).unwrap_err().status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        relations::query(&s, &manager, &Query::default())
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
}

#[test]
fn aggregate_cache_is_database_scoped_and_invalidated_by_metadata() {
    let first = state();
    let second = state();
    for id in ["a", "b"] {
        user(&first.conn().unwrap(), id, "seeker", id);
        user(&second.conn().unwrap(), id, "employer", id);
    }
    let a = query(&first, &Query::default());
    let b = query(&second, &Query::default());
    assert_eq!(a["data_rev"], b["data_rev"]);
    assert_eq!(a["query_key"], b["query_key"]);
    assert_eq!(a["summary"]["seekers"], 2);
    assert_eq!(b["summary"]["seekers"], 0);
    assert_eq!(query(&first, &Query::default()), a);
    first
        .conn()
        .unwrap()
        .execute("UPDATE users SET role='employer' WHERE id='a'", [])
        .unwrap();
    let refreshed = query(&first, &Query::default());
    assert_ne!(refreshed["data_rev"], a["data_rev"]);
    assert_eq!(refreshed["summary"]["seekers"], 1);
    assert_eq!(query(&second, &Query::default()), b);
}

#[test]
fn overflow_groups_are_disjoint_exact_and_filterable_without_fabricated_job_industry() {
    let s = state();
    {
        let db = s.conn().unwrap();
        user(&db, "owner", "employer", "Employer");
        job(&db, "job", "owner", "published");
        for i in 0..32 {
            let id = format!("s{i:02}");
            user(&db, &id, "seeker", &id);
            mandate(&db, &id, &format!("industry {i:02}"), "Rust, Go");
            matched(&db, &format!("m{i}"), &id, "job", "introduced");
        }
    }
    let grouping = Grouping {
        left: LeftGrouping::Industry,
        right: RightGrouping::Owner,
    };
    let v = query(
        &s,
        &Query {
            grouping: grouping.clone(),
            ..Default::default()
        },
    );
    let left = v["groups"]["left"].as_array().unwrap();
    assert_eq!(left.len(), 20);
    assert_eq!(
        left.iter()
            .map(|g| g["count"].as_i64().unwrap())
            .sum::<i64>(),
        33
    );
    assert_eq!(
        left.iter()
            .map(|g| g["edges"].as_i64().unwrap())
            .sum::<i64>(),
        32
    );
    assert_eq!(
        v["bundles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["count"].as_i64().unwrap())
            .sum::<i64>(),
        32
    );
    let other = left
        .iter()
        .find(|g| g["id"].as_str().unwrap().ends_with(":other"))
        .unwrap();
    let filtered = query(
        &s,
        &Query {
            mode: Mode::Nodes,
            side: Some(Side::Left),
            grouping: grouping.clone(),
            filters: Filters {
                left_group_ids: Some(vec![other["id"].as_str().unwrap().to_owned()]),
                ..Default::default()
            },
            limit: Some(100),
            ..Default::default()
        },
    );
    assert_eq!(filtered["total"], other["count"]);
    assert!(
        items(&filtered)
            .iter()
            .all(|n| n["group_id"] == other["id"])
    );
    assert_eq!(v["groups"]["right"].as_array().unwrap().len(), 1);
    assert_eq!(v["groups"]["right"][0]["label"], "Employer");
    let bad = Query {
        grouping,
        filters: Filters {
            left_group_ids: Some(vec!["left:industry:invented".into()]),
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(
        relations::query(&s, &admin(), &bad).unwrap_err().status,
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn filtered_focus_is_reported_without_silently_clearing_filters() {
    let s = fixture();
    let q = Query {
        mode: Mode::Neighbors,
        focus: Some(Focus {
            side: Side::Left,
            id: "a".into(),
        }),
        filters: Filters {
            role: Some("employer".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let error = relations::query(&s, &admin(), &q).unwrap_err();
    assert_eq!(error.status, StatusCode::CONFLICT);
    assert!(error.message.contains("excluded_by_filter"));
    assert_eq!(
        relations::query(
            &s,
            &admin(),
            &Query {
                focus: Some(Focus {
                    side: Side::Left,
                    id: "absent".into()
                }),
                ..q
            }
        )
        .unwrap_err()
        .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn http_query_is_read_only_operator_only_and_revocation_takes_effect() {
    let s = fixture();
    let raw = format!("hf_admin_{}", "a".repeat(64));
    s.conn().unwrap().execute("INSERT INTO admin_api_tokens(id,name,token_hash,created_at,expires_at) VALUES('rel-token','fixture',?1,?2,?3)",params![auth::hash_token(&raw),hireferry::now(),hireferry::now()+3600]).unwrap();
    let app = hireferry::app(s.clone());
    let request = || {
        Request::builder()
            .method("POST")
            .uri("/api/admin/relations/query")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {raw}"))
            .body(Body::from(r#"{"mode":"overview"}"#))
            .unwrap()
    };
    let before = relations::revision(&s.conn().unwrap()).unwrap();
    let response = app.clone().oneshot(request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["summary"]["edges"], 4);
    assert_eq!(relations::revision(&s.conn().unwrap()).unwrap(), before);
    s.conn()
        .unwrap()
        .execute(
            "UPDATE admin_api_tokens SET revoked=1 WHERE id='rel-token'",
            [],
        )
        .unwrap();
    assert_eq!(
        app.oneshot(request()).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

fn dense_fixture(side: usize) -> AppState {
    let s = state();
    {
        let mut db = s.conn().unwrap();
        let tx = db.transaction().unwrap();
        user(&tx, "owner", "employer", "Employer");
        for i in 0..side {
            let id = format!("s{i:04}");
            user(&tx, &id, "seeker", &id);
            mandate(&tx, &id, &format!("industry {}", i % 8), "Engineer");
            job(&tx, &format!("j{i:04}"), "owner", "published");
        }
        // Exercise normal revision and FTS triggers as well as the real schema.
        tx.execute("INSERT INTO matches(id,seeker_id,job_id,status,note,created_at,updated_at) SELECT u.id||'-'||j.id,u.id,j.id,'introduced','',1000,1000 FROM users u CROSS JOIN jobs j WHERE u.role='seeker'",[]).unwrap();
        tx.commit().unwrap();
    }
    s
}

#[test]
fn dense_query_returns_exact_aggregates_and_bounded_pages() {
    let s = dense_fixture(80);
    let v = query(
        &s,
        &Query {
            grouping: Grouping {
                left: LeftGrouping::Industry,
                right: RightGrouping::Owner,
            },
            ..Default::default()
        },
    );
    assert_eq!(v["summary"]["edges"], 6400);
    assert_eq!(v["summary"]["matched_users"], 80);
    assert_eq!(v["summary"]["matched_jobs"], 80);
    assert_eq!(v["summary"]["possible_pairs"], 6400);
    assert!(v["bundles"].as_array().unwrap().len() <= 20 * 20);
    let e = query(
        &s,
        &Query {
            mode: Mode::Edges,
            limit: Some(100),
            ..Default::default()
        },
    );
    assert_eq!(e["total"], 6400);
    assert_eq!(items(&e).len(), 100);
    assert_eq!(e["remaining"], 6300);
    assert!(e.to_string().len() < 100_000);
    let n = query(
        &s,
        &Query {
            mode: Mode::Neighbors,
            focus: Some(Focus {
                side: Side::Right,
                id: "j0000".into(),
            }),
            limit: Some(7),
            ..Default::default()
        },
    );
    assert_eq!(n["total"], 80);
    assert_eq!(items(&n).len(), 7);
    assert_eq!(n["remaining"], 73);
}

#[test]
#[ignore = "explicit 1000 x 1000 aggregate benchmark; run with --ignored --nocapture"]
fn million_edge_benchmark() {
    let fixture_start = Instant::now();
    let s = dense_fixture(1000);
    let fixture_ms = fixture_start.elapsed().as_millis();
    let start = Instant::now();
    let overview_query = Query {
        grouping: Grouping {
            left: LeftGrouping::Industry,
            right: RightGrouping::Owner,
        },
        ..Default::default()
    };
    let overview = query(&s, &overview_query);
    let overview_ms = start.elapsed().as_millis();
    let start = Instant::now();
    assert_eq!(query(&s, &overview_query), overview);
    let overview_cached_us = start.elapsed().as_micros();
    assert_eq!(overview["summary"]["edges"], 1_000_000);
    assert_eq!(overview["summary"]["matched_users"], 1000);
    assert_eq!(overview["summary"]["matched_jobs"], 1000);
    assert_eq!(overview["summary"]["possible_pairs"], 1_000_000);
    assert_eq!(
        overview["bundles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["count"].as_i64().unwrap())
            .sum::<i64>(),
        1_000_000
    );
    let start = Instant::now();
    let neighbors = query(
        &s,
        &Query {
            mode: Mode::Neighbors,
            focus: Some(Focus {
                side: Side::Right,
                id: "j0000".into(),
            }),
            limit: Some(100),
            ..Default::default()
        },
    );
    let neighbors_ms = start.elapsed().as_millis();
    assert_eq!(neighbors["total"], 1000);
    assert_eq!(items(&neighbors).len(), 100);
    assert_eq!(neighbors["remaining"], 900);
    let start = Instant::now();
    let edges = query(
        &s,
        &Query {
            mode: Mode::Edges,
            limit: Some(100),
            ..Default::default()
        },
    );
    let edges_ms = start.elapsed().as_millis();
    assert_eq!(edges["total"], 1_000_000);
    assert_eq!(items(&edges).len(), 100);
    println!(
        "relations benchmark: 1000 seekers x 1000 jobs, 1,000,000 edges; fixture={fixture_ms}ms overview_cold={overview_ms}ms overview_cached={overview_cached_us}us neighbors100={neighbors_ms}ms edges100={edges_ms}ms; overview_bytes={} neighbor_bytes={} edge_bytes={}",
        overview.to_string().len(),
        neighbors.to_string().len(),
        edges.to_string().len()
    );
}
