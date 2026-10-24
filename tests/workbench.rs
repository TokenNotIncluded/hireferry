//! Workbench query correctness and a reproducible, explicitly ignored 100k
//! SQLite benchmark. Every identity, grant and document is a synthetic fixture.
use axum::http::StatusCode;
use hireferry::{
    AppState,
    access::Principal,
    config::Config,
    db,
    workbench::{self, CandidateQuery, JobQuery, MatchQuery},
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{collections::HashSet, time::Instant};

fn state() -> AppState {
    AppState::new(Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    })
    .unwrap()
}
fn admin() -> Principal {
    Principal {
        user_id: None,
        admin: true,
        via_ai: false,
        actor: "admin".into(),
    }
}
fn user(db: &Connection, id: &str, role: &str, name: &str, resume: &str) {
    db.execute("INSERT INTO users(id,email,name,role,employer_paid,resume_md,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,1000,1000)",params![id,format!("{id}@example.invalid"),name,role,role=="employer",resume]).unwrap();
}
fn mandate(
    db: &Connection,
    id: &str,
    industry: &str,
    roles: &str,
    location: &str,
    min: Option<i64>,
    max: Option<i64>,
) {
    db.execute("INSERT INTO mandates(user_id,target_industry,target_roles,location,preferences,salary_min_cents,salary_max_cents,created_at,updated_at) VALUES(?1,?2,?3,?4,'Private preferences',?5,?6,1000,1000)",params![id,industry,roles,location,min,max]).unwrap();
}
fn job(db: &Connection, id: &str, owner: &str, title: &str, paid: Option<&str>) {
    db.execute("INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,headcount,status,created_at,updated_at) VALUES(?1,?2,?3,'Private job description',500000,2,'published',1000,1000)",params![id,owner,title]).unwrap();
    if let Some(status) = paid {
        db.execute("INSERT INTO orders(id,user_id,kind,job_id,amount_cents,provider,status,created_at,paid_at) VALUES(?1,?2,'publish',?3,50000,'stripe',?4,1000,1000)",params![format!("order-{id}"),owner,id,status]).unwrap();
        if matches!(status, "paid" | "partially_refunded") {
            db.execute("INSERT INTO ledger(order_id,provider,payment_id,amount_cents,currency,created_at) VALUES(?1,'stripe',?2,50000,'CNY',1000)",params![format!("order-{id}"),format!("payment-{id}")]).unwrap();
        }
    }
}
fn introduction(db: &Connection, id: &str, seeker: &str, job: &str, status: &str) {
    db.execute("INSERT INTO matches(id,seeker_id,job_id,status,note,created_at,updated_at) VALUES(?1,?2,?3,?4,'Private follow-up',1000,1000)",params![id,seeker,job,status]).unwrap();
}
fn ids(v: &Value) -> HashSet<String> {
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn graph_scope_is_the_intersection_of_visible_seekers_and_jobs() {
    let s = state();
    {
        let db = s.conn().unwrap();
        user(&db, "owner", "employer", "Owner", "");
        for seeker in ["a", "b", "outside"] {
            user(&db, seeker, "seeker", seeker, "");
        }
        for role in ["role-a", "role-b", "outside-role"] {
            job(&db, role, "owner", role, Some("paid"));
        }
        introduction(&db, "a-a", "a", "role-a", "introduced");
        introduction(&db, "b-b", "b", "role-b", "declined");
        introduction(&db, "outside-person", "outside", "role-a", "hired");
        introduction(&db, "outside-job", "a", "outside-role", "interview");
    }
    let q = MatchQuery {
        seeker_ids: Some("b,a,a".into()),
        job_ids: Some("role-b,role-a".into()),
        limit: Some(1),
        ..Default::default()
    };
    let first = workbench::search_matches(&s, &admin(), &q).unwrap();
    assert_eq!(first["total"], 2);
    assert_eq!(
        first["stage_counts"],
        json!({"introduced":1,"interview":0,"hired":0,"declined":1})
    );
    let second = workbench::search_matches(
        &s,
        &admin(),
        &MatchQuery {
            seeker_ids: Some("a,b".into()),
            job_ids: Some("role-a,role-b".into()),
            cursor: first["next_cursor"].as_str().map(str::to_owned),
            ..q.clone()
        },
    )
    .unwrap();
    assert!(ids(&first).is_disjoint(&ids(&second)));
    assert_eq!(
        ids(&first)
            .union(&ids(&second))
            .cloned()
            .collect::<HashSet<_>>(),
        HashSet::from(["a-a".into(), "b-b".into()])
    );
    let changed = MatchQuery {
        seeker_ids: Some("a".into()),
        cursor: first["next_cursor"].as_str().map(str::to_owned),
        ..q.clone()
    };
    assert!(workbench::search_matches(&s, &admin(), &changed).is_err());
    let empty = workbench::search_matches(
        &s,
        &admin(),
        &MatchQuery {
            seeker_ids: Some(String::new()),
            ..q.clone()
        },
    )
    .unwrap();
    assert_eq!(empty["total"], 0);
    let status = workbench::search_matches(
        &s,
        &admin(),
        &MatchQuery {
            status: Some("declined".into()),
            ..q
        },
    )
    .unwrap();
    assert_eq!(status["total"], 1);
    assert_eq!(status["items"][0]["id"], "b-b");
}

#[test]
fn graph_scope_is_bounded_parameterized_and_still_operator_only() {
    let s = state();
    for invalid in [
        "a,,b".to_owned(),
        "x".repeat(81),
        "a\n".to_owned(),
        vec!["a"; 201].join(","),
    ] {
        assert!(
            workbench::search_matches(
                &s,
                &admin(),
                &MatchQuery {
                    seeker_ids: Some(invalid),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    let query = MatchQuery {
        job_ids: Some("'); DROP TABLE jobs;--".into()),
        ..Default::default()
    };
    assert_eq!(
        workbench::search_matches(&s, &admin(), &query).unwrap()["total"],
        0
    );
    assert!(s.conn().unwrap().prepare("SELECT id FROM jobs").is_ok());
    let ordinary = Principal {
        user_id: Some("customer".into()),
        admin: false,
        via_ai: false,
        actor: "customer".into(),
    };
    assert!(workbench::search_matches(&s, &ordinary, &query).is_err());
}

#[test]
fn workbench_default_capacity_is_configurable_and_query_limit_still_overrides_it() {
    let mut cfg = Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    };
    cfg.limits.workbench_page_size = 2;
    let s = AppState::new(cfg).unwrap();
    {
        let db = s.conn().unwrap();
        user(&db, "owner", "employer", "Owner", "");
        for n in 1..=3 {
            let seeker = format!("seeker-{n}");
            let role = format!("job-{n}");
            user(&db, &seeker, "seeker", &seeker, "Private resume");
            mandate(
                &db,
                &seeker,
                "Engineering",
                "Developer",
                "Remote",
                None,
                None,
            );
            job(&db, &role, "owner", &role, Some("paid"));
            introduction(&db, &format!("match-{n}"), &seeker, &role, "introduced");
        }
    }
    for page in [
        workbench::search_jobs(&s, &admin(), &JobQuery::default()).unwrap(),
        workbench::search_candidates(&s, &admin(), &CandidateQuery::default()).unwrap(),
        workbench::search_matches(&s, &admin(), &MatchQuery::default()).unwrap(),
    ] {
        assert_eq!(page["total"], 3);
        assert_eq!(page["limit"], 2);
        assert_eq!(page["items"].as_array().unwrap().len(), 2);
        assert!(page["next_cursor"].is_string());
    }
    for page in [
        workbench::search_jobs(
            &s,
            &admin(),
            &JobQuery {
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap(),
        workbench::search_candidates(
            &s,
            &admin(),
            &CandidateQuery {
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap(),
        workbench::search_matches(
            &s,
            &admin(),
            &MatchQuery {
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap(),
    ] {
        assert_eq!(page["total"], 3);
        assert_eq!(page["limit"], 1);
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
    }
}

#[test]
fn salary_filters_compare_only_the_selected_or_job_currency() {
    let s = state();
    {
        let db = s.conn().unwrap();
        user(&db, "owner", "employer", "Owner", "");
        job(&db, "usd-job", "owner", "USD role", Some("paid"));
        db.execute("UPDATE jobs SET currency='USD' WHERE id='usd-job'", [])
            .unwrap();
        for currency in ["CNY", "USD", "JPY"] {
            user(&db, currency, "seeker", currency, "Private resume");
            mandate(
                &db,
                currency,
                "Engineering",
                "Developer",
                "Remote",
                Some(300000),
                Some(600000),
            );
            db.execute(
                "UPDATE mandates SET salary_currency=?1 WHERE user_id=?1",
                [currency],
            )
            .unwrap();
        }
    }
    let search =
        |query: CandidateQuery| workbench::search_candidates(&s, &admin(), &query).unwrap();
    let all = search(CandidateQuery::default());
    assert_eq!(all["total"], 3);
    let query = || CandidateQuery {
        salary_min_cents: Some(350000),
        salary_max_cents: Some(500000),
        ..Default::default()
    };
    assert_eq!(ids(&search(query())), HashSet::from(["CNY".to_owned()]));
    assert_eq!(
        ids(&search(CandidateQuery {
            job_id: Some("usd-job".into()),
            ..query()
        })),
        HashSet::from(["USD".to_owned()])
    );
    let yen = search(CandidateQuery {
        salary_currency: Some("JPY".into()),
        ..query()
    });
    assert_eq!(ids(&yen), HashSet::from(["JPY".to_owned()]));
    assert_eq!(yen["items"][0]["salary_currency"], "JPY");
    assert_eq!(yen["items"][0]["currency_decimals"], 0);
    assert!(
        workbench::search_candidates(
            &s,
            &admin(),
            &CandidateQuery {
                salary_currency: Some("not-a-currency".into()),
                ..query()
            }
        )
        .is_err()
    );
}

fn remove_workbench_indexes(db: &Connection) {
    for index in [
        "wb_users_fts",
        "wb_jobs_fts",
        "wb_mandates_fts",
        "wb_matches_fts",
    ] {
        db.execute_batch(&format!("DROP TRIGGER IF EXISTS {index}_insert; DROP TRIGGER IF EXISTS {index}_update; DROP TRIGGER IF EXISTS {index}_delete; DROP TABLE IF EXISTS {index};")).unwrap();
    }
    let names = db
        .prepare("SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'wb_%'")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    for name in names {
        db.execute_batch(&format!("DROP INDEX {name};")).unwrap();
    }
    // This fixture simulates a v4 database, before HumanGrant and CLI authorization.
    db.execute_batch("DROP TRIGGER cli_account_suspended; DROP TABLE cli_refresh_tokens; DROP TABLE cli_tokens; DROP TABLE cli_grants; DROP TABLE cli_authorizations;").unwrap();
    db.execute_batch("DROP TRIGGER hg_account_suspended; DROP TABLE hg_sessions; DROP TABLE hg_grants; DROP TABLE hg_requests; DROP TABLE hg_nonces; DROP TABLE hg_limits;").unwrap();
    db.pragma_update(None, "user_version", 4).unwrap();
}

#[test]
fn v4_upgrade_rebuilds_search_and_write_triggers_keep_all_four_indexes_current() {
    let s = state();
    {
        let db = s.conn().unwrap();
        remove_workbench_indexes(&db);
        user(&db, "employer", "employer", "星河研发公司", "");
        user(
            &db,
            "candidate",
            "seeker",
            "候选人甲",
            "擅长分布式计算与可靠系统",
        );
        mandate(
            &db,
            "candidate",
            "软件开发",
            "后端工程师",
            "上海浦东",
            None,
            None,
        );
        job(&db, "job", "employer", "高级工程师", Some("paid"));
        introduction(&db, "match", "candidate", "job", "declined");
        db::migrate(&db).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            18
        );
    }
    let search = |q: &str| {
        workbench::search_candidates(
            &s,
            &admin(),
            &CandidateQuery {
                q: Some(q.into()),
                ..Default::default()
            },
        )
        .unwrap()
    };
    assert_eq!(search("分布式")["total"], 1);
    assert_eq!(
        workbench::search_jobs(
            &s,
            &admin(),
            &JobQuery {
                q: Some("研发公司".into()),
                ..Default::default()
            }
        )
        .unwrap()["total"],
        1
    );
    {
        let db = s.conn().unwrap();
        db.execute(
            "UPDATE users SET resume_md='新的图形渲染技能',name='新候选人' WHERE id='candidate'",
            [],
        )
        .unwrap();
        // An UPSERT must not double-delete the old FTS row.
        db.execute("INSERT INTO mandates(user_id,target_industry,target_roles,location,preferences,created_at,updated_at) VALUES('candidate','软件开发','编译器开发','北京海淀','',1000,2000) ON CONFLICT(user_id) DO UPDATE SET target_roles=excluded.target_roles,location=excluded.location",[]).unwrap();
        db.execute(
            "UPDATE users SET name='海岸研究公司' WHERE id='employer'",
            [],
        )
        .unwrap();
        db.execute(
            "UPDATE jobs SET title='编译器负责人',jd_md='专注优化器' WHERE id='job'",
            [],
        )
        .unwrap();
        db.execute(
            "UPDATE matches SET note='安排技术沟通' WHERE id='match'",
            [],
        )
        .unwrap();
    }
    assert_eq!(search("分布式")["total"], 0);
    assert_eq!(search("图形渲染")["total"], 1);
    assert_eq!(search("编译器")["total"], 1);
    assert_eq!(
        workbench::search_jobs(
            &s,
            &admin(),
            &JobQuery {
                q: Some("研究公司".into()),
                ..Default::default()
            }
        )
        .unwrap()["total"],
        1
    );
    assert_eq!(
        workbench::search_matches(
            &s,
            &admin(),
            &MatchQuery {
                q: Some("技术沟通".into()),
                ..Default::default()
            }
        )
        .unwrap()["total"],
        1
    );
    {
        let db = s.conn().unwrap();
        db.execute("UPDATE users SET resume_md='' WHERE id='candidate'", [])
            .unwrap();
        db.execute("DELETE FROM mandates WHERE user_id='candidate'", [])
            .unwrap();
        db.execute(
            "UPDATE jobs SET status='deleted',headcount=0,title='',jd_md='' WHERE id='job'",
            [],
        )
        .unwrap();
        for index in [
            "wb_users_fts",
            "wb_jobs_fts",
            "wb_mandates_fts",
            "wb_matches_fts",
        ] {
            db.execute(
                &format!("INSERT INTO {index}({index},rank) VALUES('integrity-check',1)"),
                [],
            )
            .unwrap();
        }
        db::migrate(&db).unwrap();
    }
    assert_eq!(search("图形渲染")["total"], 0);
    assert_eq!(
        workbench::search_jobs(&s, &admin(), &JobQuery::default()).unwrap()["total"],
        0
    );
}

#[test]
fn job_queue_applies_current_paid_hiring_and_employer_state_before_pagination() {
    let s = state();
    {
        let db = s.conn().unwrap();
        user(&db, "active", "employer", "星河科技", "");
        user(&db, "banned", "employer", "封禁公司", "");
        user(&db, "unpaid", "employer", "未开通公司", "");
        user(&db, "wrong-role", "seeker", "错误身份", "");
        db.execute("UPDATE users SET banned=1 WHERE id='banned'", [])
            .unwrap();
        db.execute("UPDATE users SET employer_paid=0 WHERE id='unpaid'", [])
            .unwrap();
        for (id, owner, paid) in [
            ("paid", "active", Some("paid")),
            ("partial", "active", Some("partially_refunded")),
            ("pending", "active", Some("pending")),
            ("review", "active", Some("paid_needs_review")),
            ("refunded", "active", Some("refunded")),
            ("missing", "active", None),
            ("banned-job", "banned", Some("paid")),
            ("unpaid-job", "unpaid", Some("paid")),
            ("role-job", "wrong-role", Some("paid")),
            ("paused", "active", Some("paid")),
            ("closed", "active", Some("paid")),
            ("deleted", "active", Some("paid")),
        ] {
            job(&db, id, owner, "Rust engineer", paid);
        }
        db.execute("UPDATE jobs SET headcount=0 WHERE id='paused'", [])
            .unwrap();
        db.execute("UPDATE jobs SET status='closed' WHERE id='closed'", [])
            .unwrap();
        db.execute("UPDATE jobs SET status='deleted' WHERE id='deleted'", [])
            .unwrap();
    }
    let result = workbench::search_jobs(
        &s,
        &admin(),
        &JobQuery {
            q: Some("科技".into()),
            limit: Some(1),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result["total"], 2);
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    let next = workbench::search_jobs(
        &s,
        &admin(),
        &JobQuery {
            q: Some("科技".into()),
            limit: Some(1),
            cursor: Some(result["next_cursor"].as_str().unwrap().into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(ids(&result).is_disjoint(&ids(&next)));
    assert_eq!(next["total"], 2);
    assert!(next["next_cursor"].is_null());
    assert_eq!(
        workbench::get_job(&s, &admin(), "partial").unwrap()["employer_name"],
        "星河科技"
    );
    assert_eq!(
        workbench::get_job(&s, &admin(), "paused")
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        workbench::search_jobs(
            &s,
            &admin(),
            &JobQuery {
                employer_id: Some("banned".into()),
                ..Default::default()
            }
        )
        .unwrap()["total"],
        0
    );
}

#[test]
fn candidate_filters_overlap_unknown_salary_and_preserve_declined_history_without_rematching() {
    let s = state();
    {
        let db = s.conn().unwrap();
        user(&db, "employer", "employer", "Company", "");
        job(&db, "job", "employer", "Rust engineer", Some("paid"));
        for (id, min, max) in [
            ("eligible", Some(300000), Some(700000)),
            ("unknown", None, None),
            ("high", Some(800000), Some(900000)),
            ("paused", None, None),
            ("banned", None, None),
            ("introduced", None, None),
            ("interview", None, None),
            ("hired", None, None),
            ("declined", None, None),
        ] {
            user(&db, id, "seeker", id, "Rust and 分布式基础设施");
            mandate(&db, id, "软件开发", "Rust 后端工程师", "上海浦东", min, max);
        }
        db.execute(
            "UPDATE mandates SET status='paused' WHERE user_id='paused'",
            [],
        )
        .unwrap();
        db.execute("UPDATE users SET banned=1 WHERE id='banned'", [])
            .unwrap();
        for status in ["introduced", "interview", "hired", "declined"] {
            introduction(&db, &format!("match-{status}"), status, "job", status);
        }
    }
    let query = CandidateQuery {
        job_id: Some("job".into()),
        industry: Some("软件".into()),
        roles: Some("后端工程师".into()),
        location: Some("上海".into()),
        salary_min_cents: Some(500000),
        salary_max_cents: Some(500000),
        ..Default::default()
    };
    let result = workbench::search_candidates(&s, &admin(), &query).unwrap();
    assert_eq!(
        ids(&result),
        HashSet::from(["eligible".to_owned(), "unknown".into(), "declined".into()])
    );
    let declined = result["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == "declined")
        .unwrap();
    assert_eq!(declined["history_match"]["id"], "match-declined");
    assert_eq!(
        declined["history_match"]["conversation_id"],
        "match_match-declined"
    );
    assert_eq!(
        s.conn()
            .unwrap()
            .query_row("SELECT count(*) FROM matches", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
    let mut q = query.clone();
    q.q = Some("分布式".into());
    assert_eq!(
        workbench::search_candidates(&s, &admin(), &q).unwrap()["total"],
        3
    );
    s.conn()
        .unwrap()
        .execute("UPDATE jobs SET headcount=0 WHERE id='job'", [])
        .unwrap();
    assert_eq!(
        workbench::search_candidates(&s, &admin(), &query)
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    q.job_id = Some("missing-job".into());
    assert_eq!(
        workbench::search_candidates(&s, &admin(), &q)
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
}

#[test]
fn candidate_keysets_are_bounded_complete_and_bound_to_filters() {
    let s = state();
    {
        let mut db = s.conn().unwrap();
        let tx = db.transaction().unwrap();
        for n in 0..73 {
            let id = format!("candidate-{n:04}");
            user(&tx, &id, "seeker", &id, "Rust engineer");
            mandate(&tx, &id, "Technology", "Engineer", "Remote", None, None);
        }
        tx.commit().unwrap();
    }
    let mut seen = HashSet::new();
    let mut q = CandidateQuery::default();
    let mut first_cursor = None;
    loop {
        let result = workbench::search_candidates(&s, &admin(), &q).unwrap();
        assert_eq!(result["total"], 73);
        assert!(result["items"].as_array().unwrap().len() <= 25);
        for id in ids(&result) {
            assert!(seen.insert(id));
        }
        if first_cursor.is_none() {
            first_cursor = result["next_cursor"].as_str().map(str::to_owned);
        }
        q.cursor = result["next_cursor"].as_str().map(str::to_owned);
        if q.cursor.is_none() {
            break;
        }
        // Updating a displayed profile/mandate must not move its creation key.
        s.conn()
            .unwrap()
            .execute(
                "UPDATE mandates SET updated_at=9999 WHERE user_id='candidate-0072'",
                [],
            )
            .unwrap();
    }
    assert_eq!(seen.len(), 73);
    assert_eq!(
        workbench::search_candidates(
            &s,
            &admin(),
            &CandidateQuery {
                q: Some("changed".into()),
                cursor: first_cursor.clone(),
                ..Default::default()
            }
        )
        .unwrap_err()
        .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        workbench::search_jobs(
            &s,
            &admin(),
            &JobQuery {
                cursor: first_cursor,
                ..Default::default()
            }
        )
        .unwrap_err()
        .status,
        StatusCode::BAD_REQUEST
    );
    for limit in [0, 101, -1] {
        assert_eq!(
            workbench::search_candidates(
                &s,
                &admin(),
                &CandidateQuery {
                    limit: Some(limit),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .status,
            StatusCode::BAD_REQUEST
        );
    }
    for q in [
        CandidateQuery {
            cursor: Some("not a cursor".into()),
            ..Default::default()
        },
        CandidateQuery {
            q: Some("a".repeat(121)),
            ..Default::default()
        },
        CandidateQuery {
            q: Some("a b c d e f g h i".into()),
            ..Default::default()
        },
        CandidateQuery {
            q: Some("bad\ninput".into()),
            ..Default::default()
        },
        CandidateQuery {
            salary_min_cents: Some(-1),
            ..Default::default()
        },
        CandidateQuery {
            salary_min_cents: Some(200),
            salary_max_cents: Some(100),
            ..Default::default()
        },
    ] {
        assert_eq!(
            workbench::search_candidates(&s, &admin(), &q)
                .unwrap_err()
                .status,
            StatusCode::BAD_REQUEST
        );
    }
}

#[test]
fn followup_stage_counts_preserve_other_filters_and_latest_updates_sort_first() {
    let s = state();
    {
        let db = s.conn().unwrap();
        user(&db, "employer", "employer", "Company", "");
        user(&db, "other-employer", "employer", "Other", "");
        job(&db, "job", "employer", "Compiler engineer", Some("paid"));
        job(&db, "other-job", "other-employer", "Nurse", Some("paid"));
        for (i, status) in ["introduced", "interview", "hired", "declined", "declined"]
            .into_iter()
            .enumerate()
        {
            let id = format!("candidate-{i}");
            user(&db, &id, "seeker", &format!("Candidate {i}"), "");
            introduction(
                &db,
                &format!("match-{i}"),
                &id,
                if i == 4 { "other-job" } else { "job" },
                status,
            );
        }
        db.execute("UPDATE matches SET updated_at=9000 WHERE id='match-0'", [])
            .unwrap();
        db.execute("UPDATE users SET banned=1 WHERE id='candidate-0'", [])
            .unwrap();
        db.execute(
            "UPDATE jobs SET status='closed',headcount=0 WHERE id='job'",
            [],
        )
        .unwrap();
    }
    let q = MatchQuery {
        job_id: Some("job".into()),
        limit: Some(2),
        ..Default::default()
    };
    let first = workbench::search_matches(&s, &admin(), &q).unwrap();
    assert_eq!(first["total"], 4);
    assert_eq!(first["items"][0]["id"], "match-0");
    assert_eq!(
        first["stage_counts"],
        json!({"introduced":1,"interview":1,"hired":1,"declined":1})
    );
    let second = workbench::search_matches(
        &s,
        &admin(),
        &MatchQuery {
            cursor: first["next_cursor"].as_str().map(str::to_owned),
            ..q.clone()
        },
    )
    .unwrap();
    assert!(ids(&first).is_disjoint(&ids(&second)));
    assert_eq!(ids(&first).len() + ids(&second).len(), 4);
    let declined = workbench::search_matches(
        &s,
        &admin(),
        &MatchQuery {
            status: Some("declined".into()),
            ..q
        },
    )
    .unwrap();
    assert_eq!(declined["total"], 1);
    assert_eq!(declined["stage_counts"], first["stage_counts"]);
    assert_eq!(
        workbench::search_matches(
            &s,
            &admin(),
            &MatchQuery {
                q: Some("Nurse".into()),
                ..Default::default()
            }
        )
        .unwrap()["total"],
        1
    );
}

#[test]
fn search_syntax_is_literal_and_summaries_never_copy_full_documents_or_avatar_bytes() {
    let s = state();
    let long = "私密内容".repeat(300);
    {
        let db = s.conn().unwrap();
        user(&db, "candidate", "seeker", "Literal 100%_value", &long);
        user(&db, "other", "seeker", "Other person", "Harmless resume");
        mandate(
            &db,
            "candidate",
            "Software",
            "Rust developer",
            "Shanghai",
            None,
            None,
        );
        mandate(&db, "other", "Health", "Nurse", "Beijing", None, None);
        db.execute(
            "UPDATE users SET avatar=?1 WHERE id='candidate'",
            [format!("data:image/png;base64,{}", "A".repeat(80000))],
        )
        .unwrap();
        db.execute(
            "UPDATE mandates SET preferences=?1 WHERE user_id='candidate'",
            [&long],
        )
        .unwrap();
    }
    for term in ["%", "_", "100%", "%_value"] {
        let result = workbench::search_candidates(
            &s,
            &admin(),
            &CandidateQuery {
                q: Some(term.into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ids(&result), HashSet::from(["candidate".into()]));
    }
    for term in ["\" OR *", "') OR 1=1 --", "NEAR(foo)", "name:*"] {
        let result = workbench::search_candidates(
            &s,
            &admin(),
            &CandidateQuery {
                q: Some(term.into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result["total"], 0);
    }
    let result = workbench::search_candidates(
        &s,
        &admin(),
        &CandidateQuery {
            q: Some("Literal".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let row = &result["items"][0];
    assert_eq!(row["avatar"], "/api/avatars/candidate");
    assert_eq!(row["resume_excerpt"].as_str().unwrap().chars().count(), 240);
    assert_eq!(
        row["preferences_excerpt"].as_str().unwrap().chars().count(),
        240
    );
    assert!(row.get("resume_md").is_none() && row.get("email").is_none());
    assert!(!result.to_string().contains("data:image"));
}

fn bulk_seed(db: &mut Connection, seekers: usize, employers: usize) {
    let tx = db.transaction().unwrap();
    {
        let mut people=tx.prepare("INSERT INTO users(id,email,name,role,employer_paid,resume_md,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,1000,1000)").unwrap();
        let mut mandates=tx.prepare("INSERT INTO mandates(user_id,target_industry,target_roles,salary_min_cents,salary_max_cents,location,preferences,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,'明确委托，合成测试资料',1000,1000)").unwrap();
        for n in 0..seekers {
            let id = format!("seeker-{n:06}");
            let rust = n % 2 == 0;
            let resume = if n % 997 == 0 {
                "Rust 图形渲染与唯一目标经验"
            } else if rust {
                "Rust 分布式服务与可靠后端经验"
            } else {
                "Clinical research 医疗服务经验"
            };
            people
                .execute(params![
                    id,
                    format!("{id}@example.invalid"),
                    format!("Candidate {n:06}"),
                    "seeker",
                    false,
                    resume
                ])
                .unwrap();
            let min = if n % 4 == 0 { None } else { Some(300000_i64) };
            mandates
                .execute(params![
                    id,
                    if rust { "软件开发" } else { "医疗健康" },
                    if rust {
                        "Rust 后端工程师"
                    } else {
                        "Clinical researcher"
                    },
                    min,
                    min.map(|v| v + 500000),
                    if n % 3 == 0 {
                        "上海浦东"
                    } else {
                        "北京海淀"
                    }
                ])
                .unwrap();
        }
        for n in 0..employers {
            let id = format!("employer-{n:04}");
            people
                .execute(params![
                    id,
                    format!("{id}@example.invalid"),
                    format!("研发公司 {n:04}"),
                    "employer",
                    true,
                    ""
                ])
                .unwrap();
            job(
                &tx,
                &format!("job-{n:04}"),
                &id,
                "Rust backend engineer",
                Some("paid"),
            );
            if n < seekers {
                introduction(
                    &tx,
                    &format!("match-{n:04}"),
                    &format!("seeker-{n:06}"),
                    &format!("job-{n:04}"),
                    ["introduced", "interview", "hired", "declined"][n % 4],
                );
            }
        }
    }
    tx.commit().unwrap();
}
fn plans(db: &Connection) -> Vec<String> {
    let sql = [
        "EXPLAIN QUERY PLAN SELECT rowid FROM wb_users_fts WHERE wb_users_fts MATCH '\"唯一目标\"'",
        "EXPLAIN QUERY PLAN SELECT id FROM matches WHERE status='interview' AND (updated_at,id)<(1000,'match-0999') ORDER BY updated_at DESC,id DESC LIMIT 26",
    ];
    sql.into_iter()
        .flat_map(|sql| {
            db.prepare(sql)
                .unwrap()
                .query_map([], |r| r.get::<_, String>(3))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        })
        .collect()
}

#[test]
fn query_plans_use_keyset_and_full_text_indexes() {
    let s = state();
    let mut db = s.conn().unwrap();
    bulk_seed(&mut db, 300, 20);
    db.execute_batch("ANALYZE;").unwrap();
    let plan = plans(&db).join("\n");
    assert!(plan.contains("VIRTUAL TABLE INDEX"), "{plan}");
    assert!(plan.contains("wb_matches_stage_page"), "{plan}");
    assert!(!plan.contains("USE TEMP B-TREE"), "{plan}");
    drop(db);
    let first = workbench::search_candidates(&s, &admin(), &CandidateQuery::default()).unwrap();
    let plan = workbench::explain_candidates(
        &s,
        &admin(),
        &CandidateQuery {
            cursor: first["next_cursor"].as_str().map(str::to_owned),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(plan["strategy"], "ordered_keyset");
    let plan = plan["plan"].to_string();
    assert!(plan.contains("wb_mandates_page"), "{plan}");
    assert!(plan.contains("wb_active_seekers"), "{plan}");
    assert!(
        !plan.contains("SCAN u") && !plan.contains("USE TEMP B-TREE"),
        "{plan}"
    );
}

#[test]
#[ignore = "100,000 synthetic seekers: explicit local/CI scalability measurement"]
fn hundred_thousand_seeker_workbench_benchmark() {
    let s = state();
    let start = Instant::now();
    {
        let mut db = s.conn().unwrap();
        remove_workbench_indexes(&db);
        bulk_seed(&mut db, 100_000, 1_000);
    }
    println!(
        "WORKBENCH_SCALE fixture_seekers=100000 employers=1000 jobs=1000 matches=1000 database=memory fixture_ms={}",
        start.elapsed().as_millis()
    );
    let migration = Instant::now();
    {
        let db = s.conn().unwrap();
        db::migrate(&db).unwrap();
        db.execute_batch("ANALYZE;").unwrap();
        let page_count: i64 = db.query_row("PRAGMA page_count", [], |r| r.get(0)).unwrap();
        let page_size: i64 = db.query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap();
        println!(
            "WORKBENCH_SCALE schema5_index_build_ms={} sqlite_logical_bytes={}",
            migration.elapsed().as_millis(),
            page_count * page_size
        );
        for (n, plan) in plans(&db).iter().enumerate() {
            println!("WORKBENCH_PLAN {n} {plan}");
        }
    }
    let started = Instant::now();
    let first = workbench::search_candidates(&s, &admin(), &CandidateQuery::default()).unwrap();
    assert_eq!(first["total"], 100_000);
    assert_eq!(first["items"].as_array().unwrap().len(), 25);
    let bytes = serde_json::to_vec(&first).unwrap().len();
    assert!(
        bytes < 40_000,
        "A page must contain summaries, not full documents"
    );
    println!(
        "WORKBENCH_QUERY all_candidates_ms={} total=100000 page_rows=25 response_bytes={} candidate_row_bytes={}",
        started.elapsed().as_millis(),
        bytes,
        serde_json::to_vec(&first["items"][0]).unwrap().len()
    );
    // Explain the same prepared page SQL used by production, after ANALYZE at
    // the full cardinality. A small fixture alone cannot catch planner flips.
    let plan = workbench::explain_candidates(
        &s,
        &admin(),
        &CandidateQuery {
            cursor: first["next_cursor"].as_str().map(str::to_owned),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(plan["strategy"], "ordered_keyset");
    let plan_text = plan["plan"].to_string();
    assert!(plan_text.contains("wb_mandates_page"), "{plan_text}");
    assert!(plan_text.contains("wb_active_seekers"), "{plan_text}");
    assert!(
        !plan_text.contains("SCAN u") && !plan_text.contains("USE TEMP B-TREE"),
        "{plan_text}"
    );
    for (n, step) in plan["plan"].as_array().unwrap().iter().enumerate() {
        println!(
            "WORKBENCH_PRODUCTION_PAGE_PLAN {n} {}",
            step.as_str().unwrap()
        );
    }
    let mut q = CandidateQuery {
        limit: Some(100),
        ..Default::default()
    };
    let mut seen = HashSet::new();
    let paging = Instant::now();
    for _ in 0..10 {
        let result = workbench::search_candidates(&s, &admin(), &q).unwrap();
        assert_eq!(result["total"], 100_000);
        assert_eq!(result["items"].as_array().unwrap().len(), 100);
        for id in ids(&result) {
            assert!(seen.insert(id), "Repeated keyset row");
        }
        q.cursor = result["next_cursor"].as_str().map(str::to_owned);
        assert!(q.cursor.is_some());
    }
    assert_eq!(seen.len(), 1000);
    println!(
        "WORKBENCH_QUERY ten_keyset_pages_ms={} unique_rows=1000 page_rows=100",
        paging.elapsed().as_millis()
    );
    for (label, q, expected) in [
        (
            "chinese_fts",
            CandidateQuery {
                q: Some("唯一目标".into()),
                ..Default::default()
            },
            101,
        ),
        (
            "short_chinese_location",
            CandidateQuery {
                location: Some("北京".into()),
                ..Default::default()
            },
            66_666,
        ),
        (
            "role_salary_job_filter",
            CandidateQuery {
                job_id: Some("job-0000".into()),
                roles: Some("Rust".into()),
                salary_min_cents: Some(500000),
                salary_max_cents: Some(500000),
                ..Default::default()
            },
            49_999,
        ),
    ] {
        let started = Instant::now();
        let result = workbench::search_candidates(&s, &admin(), &q).unwrap();
        assert_eq!(result["total"], expected, "{label}");
        assert!(result["items"].as_array().unwrap().len() <= 25);
        println!(
            "WORKBENCH_QUERY {label}_ms={} total={} page_rows={}",
            started.elapsed().as_millis(),
            expected,
            result["items"].as_array().unwrap().len()
        );
        if label == "chinese_fts" {
            let plan = workbench::explain_candidates(&s, &admin(), &q).unwrap();
            assert_eq!(plan["strategy"], "selective_fts");
            assert!(plan["plan"].to_string().contains("VIRTUAL TABLE INDEX"));
        }
    }
    let started = Instant::now();
    let jobs = workbench::search_jobs(
        &s,
        &admin(),
        &JobQuery {
            q: Some("研发公司".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(jobs["total"], 1000);
    println!(
        "WORKBENCH_QUERY jobs_ms={} total=1000 page_rows={}",
        started.elapsed().as_millis(),
        jobs["items"].as_array().unwrap().len()
    );
    let started = Instant::now();
    let matches = workbench::search_matches(
        &s,
        &admin(),
        &MatchQuery {
            status: Some("interview".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(matches["total"], 250);
    assert_eq!(
        matches["stage_counts"],
        json!({"introduced":250,"interview":250,"hired":250,"declined":250})
    );
    println!(
        "WORKBENCH_QUERY followups_ms={} stage_total=250 all_stages_total=1000 page_rows={}",
        started.elapsed().as_millis(),
        matches["items"].as_array().unwrap().len()
    );
    let started = Instant::now();
    let introductions = hireferry::admin::add_matches(
        &s,
        &admin(),
        hireferry::admin::BulkMatchInput {
            job_id: "job-0000".into(),
            seeker_ids: (1..=100).map(|n| format!("seeker-{n:06}")).collect(),
            note: "Synthetic bulk operation".into(),
        },
    )
    .unwrap();
    assert_eq!(introductions["created"], 100);
    assert_eq!(introductions["reused"], 0);
    println!(
        "WORKBENCH_MUTATION bulk_introduce_ms={} created=100 transaction=atomic",
        started.elapsed().as_millis()
    );
    println!("WORKBENCH_SCALE result=PASS measurement=local_memory_sql_not_a_production_SLA");
}
