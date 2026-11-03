//! Resource history, the process table, and the inventory, against a real database.
//!
//! These prove the parts a pure function cannot: that a posted batch becomes rows,
//! that an hour of raw samples answers with at most its point budget, that a day
//! and a week answer from the rollup, that the rollup run is idempotent, that
//! partition maintenance creates the days ahead and drops the days behind, and
//! that a snapshot is stored without an environment value in it.
//!
//! They run only when `TEST_DATABASE_URL` is set and own a database of their own,
//! so they can run in parallel with the other suites without either one dropping
//! the other's schema:
//!
//!   export TEST_DATABASE_URL='postgres://core@localhost:55432/nestbox_metrics?host=/tmp/opencode/pgtest'
//!   cargo test --test metrics -- --test-threads=1

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
use tokio::sync::OnceCell;
use uuid::Uuid;

use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::domain::metrics::{
    InventoryProbe, MetricBatch, MetricRange, MetricSource, RawPoint,
};
use nestbox_control_api::domain::runtime::SandboxSpec;
use nestbox_control_api::domain::sandbox::Sandbox;
use nestbox_control_api::error::ErrorCode;
use nestbox_control_api::runtime::RuntimeAdapter;
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::inventory_service::{self, InventoryView};
use nestbox_control_api::services::metrics_service::{self, IngestOutcome};
use nestbox_control_api::services::ServiceContext;
use nestbox_control_api::store::TenantScope;
use nestbox_control_api::store::postgres::metrics as metric_store;
use nestbox_control_api::store::postgres::sandboxes as sandbox_store;

/// The database this suite owns. Fixed so a previous failed run is replaced
/// rather than accumulated.
const DATABASE: &str = "nestbox_metrics_suite";

/// The seeded cluster, node, and administrator. The audit trail and the sandbox
/// row have foreign keys, so a made-up id would turn a successful write into a 500.
const CLUSTER: &str = "00000000-0000-0000-0000-000000000004";
const NODE: &str = "00000000-0000-0000-0000-000000000003";
const ADMIN: &str = "00000000-0000-0000-0000-000000000005";

static READY: OnceCell<bool> = OnceCell::const_new();

/// Create the suite's database once per process, then migrate it.
async fn ready() -> bool {
    *READY
        .get_or_init(|| async {
            let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
                return false;
            };
            let Ok(options) = PgConnectOptions::from_str(&url) else {
                return false;
            };

            let Ok(maintenance) =
                sqlx::PgPool::connect_with(options.clone().database("postgres")).await
            else {
                return false;
            };
            let setup = async {
                sqlx::query(
                    "DO $$BEGIN
                        IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'nestbox_app') THEN
                            CREATE ROLE nestbox_app LOGIN;
                        END IF;
                    END$$",
                )
                .execute(&maintenance)
                .await?;
                sqlx::query(&format!("DROP DATABASE IF EXISTS {DATABASE} WITH (FORCE)"))
                    .execute(&maintenance)
                    .await?;
                sqlx::query(&format!("CREATE DATABASE {DATABASE}"))
                    .execute(&maintenance)
                    .await?;

                let pool = sqlx::PgPool::connect_with(options.database(DATABASE)).await?;
                sqlx::query("GRANT USAGE, CREATE ON SCHEMA public TO nestbox_app")
                    .execute(&pool)
                    .await?;
                sqlx::migrate!("./migrations").run(&pool).await?;
                Ok::<(), sqlx::Error>(())
            }
            .await;
            setup.is_ok()
        })
        .await
}

async fn pool() -> Option<PgPool> {
    if !ready().await {
        return None;
    }
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let options = PgConnectOptions::from_str(&url).ok()?;
    sqlx::PgPool::connect_with(options.database(DATABASE))
        .await
        .ok()
}

async fn environment_id(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM environments WHERE slug = 'default'")
        .fetch_one(pool)
        .await
        .expect("the default environment is seeded")
}

fn principal(environment: Uuid, role: Role) -> Principal {
    Principal {
        user_id: Uuid::parse_str(ADMIN).expect("a uuid"),
        environment_id: environment,
        role,
        display_name: "Test operator".into(),
        via: CredentialSource::Token,
    }
}

fn context(pool: &PgPool, runtime: &Arc<FakeRuntime>) -> ServiceContext {
    ServiceContext::new(
        pool.clone(),
        runtime.clone(),
        Arc::new(nestbox_control_api::events::bus::LocalBus::new()),
        std::path::PathBuf::from("/tmp/opencode"),
    )
}

fn unique(prefix: &str) -> String {
    let suffix = Uuid::new_v4().simple().to_string();
    format!("{prefix}-{}", &suffix[..8])
}

/// A sandbox that exists both in the database and in the fake runner.
///
/// Both halves matter: the service resolves the name through the runner, and the
/// metrics and inventory rows hang off the database row.
async fn sandbox(pool: &PgPool, environment: Uuid, runtime: &FakeRuntime) -> Sandbox {
    let name = unique("metrics");
    runtime
        .create(SandboxSpec {
            name: name.clone(),
            image: "nestbox/tool-hermes@sha256:0123456789abcdef".into(),
            command: vec!["/usr/local/bin/hermes".into(), "serve".into()],
            environment: Vec::new(),
            pack_key: "hermes".into(),
            tool_profile: "base".into(),
            memory_limit: "1Gi".into(),
            cpu_limit: 1.0,
            pids_limit: 256,
            mounts: Vec::new(),
            gpu_count: 0,
        egress_mode: "forward_proxy".into(),
        node_id: None,
            memory_reservation: None,
            cpu_shares: None,
        })
        .await
        .expect("the fake runner creates a sandbox");

    let mut scope = TenantScope::begin(pool, environment)
        .await
        .expect("a tenant scope");
    let row = sandbox_store::insert(
        &mut scope,
        sandbox_store::NewSandbox {
            cluster_id: Uuid::parse_str(CLUSTER).expect("a uuid"),
            gateway_id: None,
            node_id: Uuid::parse_str(NODE).expect("a uuid"),
            name: &name,
            pack_key: "hermes",
            image: "nestbox/tool-hermes@sha256:0123456789abcdef",
            image_digest: Some("sha256:0123456789abcdef"),
            spec: &serde_json::json!({}),
            parent_id: None,
            owner_user_id: None,
        },
    )
    .await
    .expect("the sandbox row is inserted");
    scope.commit().await.expect("the insert commits");
    row
}

/// One sample for a sandbox, a plausible container.
fn point(sandbox: &str, ts: DateTime<Utc>, cpu: f32) -> RawPoint {
    RawPoint {
        sandbox: sandbox.to_string(),
        ts,
        cpu_pct: cpu,
        mem_bytes: 128 * 1024 * 1024,
        mem_limit: 1024 * 1024 * 1024,
        net_rx: 1_024,
        net_tx: 2_048,
        blk_read: 4_096,
        blk_write: 8_192,
        pids: 12,
    }
}

async fn raw_rows(pool: &PgPool, sandbox_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM sandbox_metrics WHERE sandbox_id = $1")
        .bind(sandbox_id)
        .fetch_one(pool)
        .await
        .expect("the raw table is readable")
}

async fn ingest_all(
    ctx: &ServiceContext,
    who: &Principal,
    points: Vec<RawPoint>,
) -> IngestOutcome {
    let mut outcome = IngestOutcome::default();
    for chunk in points.chunks(MetricBatch::MAX_SAMPLES) {
        let batch = MetricBatch {
            samples: chunk.to_vec(),
        };
        let part = metrics_service::ingest(ctx, who, batch)
            .await
            .expect("a plausible batch is accepted");
        outcome.stored += part.stored;
        outcome.dropped_out_of_window += part.dropped_out_of_window;
        outcome.dropped_implausible += part.dropped_implausible;
        outcome.dropped_unknown_sandbox += part.dropped_unknown_sandbox;
    }
    outcome
}

#[tokio::test]
async fn a_batch_becomes_rows_and_the_hour_window_reads_it_within_its_budget() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;
    let environment = environment_id(pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(pool, &runtime);
    let who = principal(environment, Role::Operator);
    let sandbox = sandbox(pool, environment, &runtime).await;

    // Fifteen minutes at one second: more samples than an hour's budget allows,
    // so the answer must be grouped rather than echoed.
    let start = Utc::now() - Duration::minutes(20);
    let points: Vec<RawPoint> = (0..900)
        .map(|index| {
            point(
                &sandbox.name,
                start + Duration::seconds(index),
                10.0 + (index % 5) as f32,
            )
        })
        .collect();

    let outcome = ingest_all(&ctx, &who, points).await;
    assert_eq!(outcome.stored, 900, "{outcome:?}");
    assert_eq!(outcome.dropped_unknown_sandbox, 0);
    assert_eq!(raw_rows(pool, sandbox.id).await, 900);

    let series = metrics_service::series(&ctx, &who, sandbox.id, MetricRange::OneHour, None)
        .await
        .expect("the hour window answers");
    assert_eq!(series.source, MetricSource::Raw);
    assert_eq!(series.step_seconds, 5);
    assert!(
        series.points.len() <= MetricRange::OneHour.max_points(),
        "{} points exceed the budget",
        series.points.len()
    );
    // 900 s / 5 s = 180 buckets, minus the partial ones at either end.
    assert!(
        (170..=181).contains(&series.points.len()),
        "expected about 180 buckets, got {}",
        series.points.len()
    );
    for bucket in &series.points {
        assert!(
            (1..=5).contains(&bucket.samples),
            "a five second bucket held {} samples",
            bucket.samples
        );
        assert_eq!(bucket.sandbox_id, sandbox.id);
        assert!(bucket.cpu_pct_avg >= 10.0 && bucket.cpu_pct_avg <= 14.0);
    }
}

#[tokio::test]
async fn a_point_older_than_the_retention_is_dropped_instead_of_written() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;
    let environment = environment_id(pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(pool, &runtime);
    let who = principal(environment, Role::Operator);
    let sandbox = sandbox(pool, environment, &runtime).await;

    let stale = point(
        &sandbox.name,
        Utc::now() - Duration::hours(25),
        5.0,
    );
    let future = point(&sandbox.name, Utc::now() + Duration::minutes(30), 5.0);
    let fresh = point(&sandbox.name, Utc::now() - Duration::seconds(30), 5.0);

    let outcome = ingest_all(&ctx, &who, vec![stale, future, fresh]).await;
    assert_eq!(outcome.stored, 1);
    assert_eq!(outcome.dropped_out_of_window, 2);
    // The stale point did not create a partition of its own: only the fresh row
    // is in the table.
    assert_eq!(raw_rows(pool, sandbox.id).await, 1);
}

#[tokio::test]
async fn a_batch_names_sandboxes_and_only_this_environments_are_stored() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;
    let environment = environment_id(pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(pool, &runtime);
    let who = principal(environment, Role::Operator);
    let sandbox = sandbox(pool, environment, &runtime).await;

    let now = Utc::now() - Duration::seconds(10);
    let outcome = ingest_all(
        &ctx,
        &who,
        vec![
            point(&sandbox.name, now, 1.0),
            point("no-such-sandbox", now, 1.0),
        ],
    )
    .await;

    assert_eq!(outcome.stored, 1);
    assert_eq!(outcome.dropped_unknown_sandbox, 1);
    assert_eq!(raw_rows(pool, sandbox.id).await, 1);
}

#[tokio::test]
async fn the_day_and_week_read_the_rollup_and_a_second_run_changes_nothing() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;
    let environment = environment_id(pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(pool, &runtime);
    let who = principal(environment, Role::Operator);
    let sandbox = sandbox(pool, environment, &runtime).await;

    // Two seconds apart for ten minutes, ending two minutes ago so every minute
    // of the window is complete.
    let start = Utc::now() - Duration::minutes(12);
    let points: Vec<RawPoint> = (0..300)
        .map(|index| {
            point(
                &sandbox.name,
                start + Duration::seconds(index * 2),
                20.0 + (index % 10) as f32,
            )
        })
        .collect();
    ingest_all(&ctx, &who, points).await;

    let first = metrics_service::rollup(&ctx, environment, 60, 7)
        .await
        .expect("the rollup runs");
    assert!(
        first["rows_written"].as_u64().unwrap_or(0) >= 10,
        "ten minutes should produce at least ten buckets: {first}"
    );

    let buckets = |pool: PgPool, sandbox_id: Uuid| async move {
        sqlx::query_as::<_, (DateTime<Utc>, f32, i32)>(
            "SELECT bucket, cpu_pct_avg, samples
               FROM sandbox_metrics_1m
              WHERE sandbox_id = $1
              ORDER BY bucket",
        )
        .bind(sandbox_id)
        .fetch_all(&pool)
        .await
        .expect("the rollup is readable")
    };
    let before = buckets(pool.clone(), sandbox.id).await;
    assert!(!before.is_empty());

    let _second = metrics_service::rollup(&ctx, environment, 60, 7)
        .await
        .expect("the rollup runs again");
    let after = buckets(pool.clone(), sandbox.id).await;
    assert_eq!(before, after, "a second run must leave the same rows");

    let day = metrics_service::series(&ctx, &who, sandbox.id, MetricRange::OneDay, None)
        .await
        .expect("the day window answers");
    assert_eq!(day.source, MetricSource::Rollup);
    assert_eq!(day.step_seconds, 60);
    assert!(!day.points.is_empty());
    assert!(day.points.len() <= MetricRange::OneDay.max_points());

    let week = metrics_service::series(&ctx, &who, sandbox.id, MetricRange::SevenDays, None)
        .await
        .expect("the week window answers");
    assert_eq!(week.source, MetricSource::Rollup);
    assert!(!week.points.is_empty());
    assert!(week.points.len() <= MetricRange::SevenDays.max_points());
}

#[tokio::test]
async fn partition_maintenance_creates_the_days_ahead_and_drops_the_days_behind() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;

    let created = metric_store::ensure_partitions(pool, 2)
        .await
        .expect("the partitions are created");
    assert_eq!(created.len(), 3, "today and two days ahead: {created:?}");
    let today = Utc::now().date_naive();
    assert!(
        created
            .iter()
            .any(|name| name.ends_with(&today.format("%Y%m%d").to_string())),
        "today's partition should be among {created:?}"
    );
    let ahead = metric_store::ensure_partitions(pool, 2)
        .await
        .expect("a second run is a no-op");
    assert_eq!(ahead, created, "creating an existing partition is idempotent");

    // A day that arrived late: the partition is created afterwards, and the row
    // that was already in the default partition moves into it.
    let behind = today - Duration::days(3);
    let name = metric_store::create_day_partition(pool, behind)
        .await
        .expect("an old day can be created on demand");
    let environment = environment_id(pool).await;
    sqlx::query(
        "INSERT INTO sandbox_metrics (sandbox_id, environment_id, ts, cpu_pct)
         VALUES ($1, $2, $3, 1.0)",
    )
    .bind(Uuid::new_v4())
    .bind(environment)
    .bind(behind.and_hms_opt(12, 0, 0).expect("a valid time").and_utc())
    .execute(pool)
    .await
    .expect("the row lands in a partition");

    let dropped = metric_store::drop_expired_partitions(pool, 1)
        .await
        .expect("the retention runs");
    assert!(dropped.contains(&name), "{dropped:?} should contain {name}");
    let still_there: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
        .bind(format!("public.{name}"))
        .fetch_one(pool)
        .await
        .expect("the catalog answers");
    assert!(still_there.is_none(), "{name} should be gone");

    // Today's partition survives: it is inside the retention.
    let today_name = format!("sandbox_metrics_{}", today.format("%Y%m%d"));
    let survives: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
        .bind(format!("public.{today_name}"))
        .fetch_one(pool)
        .await
        .expect("the catalog answers");
    assert!(survives.is_some(), "{today_name} must not be dropped");
}

#[tokio::test]
async fn the_process_table_comes_from_the_runner() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;
    let environment = environment_id(pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(pool, &runtime);
    let who = principal(environment, Role::Viewer);
    let sandbox = sandbox(pool, environment, &runtime).await;

    let table = metrics_service::processes(&ctx, &who, sandbox.id)
        .await
        .expect("a running sandbox has a process table");
    assert_eq!(table.items.len(), 2);
    assert_eq!(table.items[0].pid, 1);
    assert_eq!(table.items[0].ppid, None);
    assert!(table.items[0].command.contains("hermes"));
    assert!(!table.truncated);
    assert!(table.columns.contains(&"COMMAND".to_string()));

    // A stopped sandbox answers 409 rather than an empty list, so the console can
    // say why the table is not there.
    runtime
        .stop(&sandbox.name)
        .await
        .expect("the fake stops the sandbox");
    let error = metrics_service::processes(&ctx, &who, sandbox.id)
        .await
        .expect_err("a stopped sandbox has no table");
    assert_eq!(error.code, ErrorCode::SANDBOX_INVALID_STATE);
}

#[tokio::test]
async fn an_inventory_snapshot_reports_os_packages_hermes_ports_and_env_names() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;
    let environment = environment_id(pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(pool, &runtime);
    let who = principal(environment, Role::Operator);
    let sandbox = sandbox(pool, environment, &runtime).await;

    let before = inventory_service::load(&ctx, &who, sandbox.id)
        .await
        .expect("a sandbox without a snapshot is not an error");
    assert!(before.inventory.is_none());
    assert!(before.age_seconds.is_none());

    let view = inventory_service::refresh(&ctx, &who, sandbox.id)
        .await
        .expect("the snapshot is taken");
    let InventoryView {
        inventory: Some(inventory),
        age_seconds: Some(age),
    } = view
    else {
        panic!("a refresh answers with a snapshot");
    };

    assert_eq!(inventory.os["ID"], "ubuntu");
    assert!(inventory.packages.as_array().is_some_and(|items| !items.is_empty()));
    assert!(
        inventory.agents["hermes"]
            .as_str()
            .unwrap_or_default()
            .contains("0.22.5"),
        "the Hermes version is the point of the tab: {:?}",
        inventory.agents
    );
    assert!(inventory
        .listening
        .as_array()
        .is_some_and(|items| items.len() == 2));
    assert!(inventory.env_keys.contains(&"PATH".to_string()));
    assert!(age >= 0);

    // Stored, so a second read answers from the database rather than the runner.
    let stored = inventory_service::load(&ctx, &who, sandbox.id)
        .await
        .expect("the snapshot is readable");
    assert_eq!(stored.inventory.expect("a stored snapshot").sandbox_id, sandbox.id);
}

#[tokio::test]
async fn a_stored_snapshot_never_carries_an_environment_value() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;
    let environment = environment_id(pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(pool, &runtime);
    let sandbox = sandbox(pool, environment, &runtime).await;

    // A probe that prints what it must not: a value with its key, a bare token,
    // and a secret in a version banner. Every route into the snapshot is tried.
    let probe = InventoryProbe {
        taken_at: Utc::now(),
        facts: serde_json::json!({
            "os": { "ID": "ubuntu", "NAME": "Ubuntu" },
            "packages": [{ "name": "bash", "version": "5.2.21" }],
            "agents": { "hermes": "0.22.5 (build sk-live-abcdefghijklmnop)" },
            "listening": [{ "address": "0.0.0.0:8080", "protocol": "tcp" }],
            "env_keys": ["PATH", "OPENAI_API_KEY=sk-live-abcdefghijklmnop", "sk-live-abcdefghijklmnop"],
            "mounts": [{ "target": "/home/nestbox", "fstype": "ext4", "source": "/dev/sda1" }],
            "image": { "reference": "nestbox/tool-hermes", "id": "sha256:00", "labels": { "nestbox.pack": "hermes" } },
        }),
        note: Some("the probe exited with status 0".into()),
    };
    let snapshot = inventory_service::sanitize(sandbox.id, &probe);
    inventory_service::store_snapshot(&ctx, environment, &snapshot)
        .await
        .expect("the snapshot is stored");

    let stored: serde_json::Value = sqlx::query_scalar(
        "SELECT to_jsonb(r) FROM (
            SELECT os, packages, agents, tool_versions, listening, env_keys, mounts, image, note
              FROM sandbox_inventory WHERE sandbox_id = $1
         ) r",
    )
    .bind(sandbox.id)
    .fetch_one(pool)
    .await
    .expect("the row is readable");
    let text = stored.to_string();
    assert!(
        !text.contains("sk-live"),
        "a secret value reached the database: {text}"
    );
    assert!(
        !text.contains("abcdefghijklmnop"),
        "a secret value reached the database: {text}"
    );
    assert_eq!(stored["env_keys"], serde_json::json!(["OPENAI_API_KEY", "PATH"]));
}

#[tokio::test]
async fn a_viewer_may_read_the_history_and_the_snapshot_but_not_post_or_refresh() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;
    let environment = environment_id(pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(pool, &runtime);
    let viewer = principal(environment, Role::Viewer);
    let sandbox = sandbox(pool, environment, &runtime).await;

    let batch = MetricBatch {
        samples: vec![point(&sandbox.name, Utc::now() - Duration::seconds(5), 1.0)],
    };
    let refused = metrics_service::ingest(&ctx, &viewer, batch)
        .await
        .expect_err("a viewer cannot post samples");
    assert_eq!(refused.code, ErrorCode::FORBIDDEN);

    let refused = inventory_service::refresh(&ctx, &viewer, sandbox.id)
        .await
        .expect_err("a viewer cannot run the probe");
    assert_eq!(refused.code, ErrorCode::FORBIDDEN);

    // Reading is allowed, and an empty series is a real answer for a sandbox
    // that has not been sampled yet.
    let series = metrics_service::series(&ctx, &viewer, sandbox.id, MetricRange::OneHour, None)
        .await
        .expect("a viewer reads the history");
    assert!(series.points.is_empty());
}
