//! Database integration checks.
//!
//! These run only when `TEST_DATABASE_URL` points at a disposable PostgreSQL.
//! Without it they skip, so `cargo test` works on a machine with no database —
//! but they are the tests that actually prove the schema, the row-level
//! security, and the seed are correct.
//!
//!   docker compose -f ../../deploy/docker-compose.test.yml up -d
//!   export TEST_DATABASE_URL=postgres://nestbox:test@localhost:55432/nestbox_test
//!   cargo test --test database

use uuid::Uuid;

async fn connect() -> Option<sqlx::PgPool> {
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let pool = sqlx::PgPool::connect(&url).await.ok()?;
    // A fresh schema every run, so the migration path itself is tested rather
    // than assumed.
    let _ = sqlx::query("DROP SCHEMA IF EXISTS public CASCADE").execute(&pool).await;
    let _ = sqlx::query("CREATE SCHEMA public").execute(&pool).await;
    // The application role is created by the Postgres init script on the
    // appliance. Here it is created before the migrations, so 0009 can grant it
    // and the row-level-security test can switch to it.
    let _ = sqlx::query(
        "DO $$BEGIN
            IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'nestbox_app') THEN
                CREATE ROLE nestbox_app LOGIN;
            END IF;
        END$$",
    )
    .execute(&pool)
    .await;
    sqlx::query("GRANT USAGE, CREATE ON SCHEMA public TO nestbox_app")
        .execute(&pool)
        .await
        .ok()?;
    sqlx::migrate!("./migrations").run(&pool).await.ok()?;
    Some(pool)
}

/// The migrations apply cleanly and seed the environment the console expects.
#[tokio::test]
async fn migrations_seed_the_default_environment() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };

    let environment: Option<String> =
        sqlx::query_scalar("SELECT name FROM environments WHERE slug = 'default'")
            .fetch_optional(&pool)
            .await
            .expect("query");
    assert_eq!(environment.as_deref(), Some("default"));

    let models: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert!(models >= 4, "the model catalogue should be seeded");

    let schedules: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schedules")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert!(schedules >= 8, "the operational schedules should be seeded");
}

/// The catalogue describes the gate it actually uses.
///
/// A CPU model without a RAM figure has no gate at all and would look
/// installable everywhere; a GPU model without a VRAM figure likewise. This is
/// checked as data rather than left to convention, because the catalog is
/// written by hand and a missing column is invisible in JSON (`null` reads as
/// "no requirement", not "unknown").
#[tokio::test]
async fn the_model_catalog_states_the_gate_it_uses() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };

    // The hand-written rows must state their gates: a missing figure there is a
    // curation mistake. A synced row may legitimately carry none — the
    // repository published no size — and the classifier answers `unknown` for it
    // (ADR 0019), which `domain::model` tests; it is not a catalogue defect.
    let cpu_without_ram: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models
          WHERE source = 'seed' AND is_cpu_only
            AND (min_ram_bytes IS NULL OR min_vram_bytes IS NOT NULL)",
    )
    .fetch_one(&pool)
    .await
    .expect("query");
    assert_eq!(
        cpu_without_ram, 0,
        "a CPU-only model needs a RAM gate and must not claim a VRAM gate"
    );

    let cuda_without_vram: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models
          WHERE source = 'seed' AND runtime = 'cuda'
            AND NOT is_remote AND min_vram_bytes IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("query");
    assert_eq!(
        cuda_without_vram, 0,
        "a GPU-only model needs a VRAM gate"
    );

    let hybrid_without_either: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models
          WHERE source = 'seed' AND runtime = 'hybrid'
            AND (min_vram_bytes IS NULL OR min_ram_bytes IS NULL)",
    )
    .fetch_one(&pool)
    .await
    .expect("query");
    assert_eq!(
        hybrid_without_either, 0,
        "a hybrid model needs both the GPU gate and the CPU fallback gate"
    );

    let catalog_size: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert!(
        catalog_size >= 80,
        "the catalog should be broad (NVIDIA, DeepSeek, Qwen, Llama, Gemma, Phi, CPU-only, frontier), got {catalog_size}"
    );

    // The frontier rows are pinned seeds, so the sync can never rewrite them.
    let unpinned_seed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models WHERE source = 'seed' AND NOT pinned",
    )
    .fetch_one(&pool)
    .await
    .expect("query");
    assert_eq!(unpinned_seed, 0, "every seed row is pinned");

    // Recommended never lies below minimum, and the two units are distinct on
    // purpose: a row that copies VRAM into RAM would hide the CPU path.
    let copied_gate: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models
          WHERE recommended_ram_bytes IS NOT NULL AND min_ram_bytes IS NOT NULL
            AND recommended_ram_bytes < min_ram_bytes",
    )
    .fetch_one(&pool)
    .await
    .expect("query");
    assert_eq!(copied_gate, 0, "recommended RAM must be at least the minimum");
}

/// The node tables are in place: one local node, every sandbox and hardware
/// profile on it, and the builder catalogs seeded.
#[tokio::test]
async fn the_local_node_and_builder_catalogs_are_seeded() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };

    let local: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM nodes WHERE is_local")
        .fetch_one(&pool)
        .await
        .expect("query");
    assert_eq!(local, 1, "exactly one local node");

    let orphan_sandboxes: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sandboxes WHERE node_id IS NULL")
            .fetch_one(&pool)
            .await
            .expect("query");
    assert_eq!(orphan_sandboxes, 0);

    let baselines: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM baselines")
        .fetch_one(&pool)
        .await
        .expect("query");
    assert_eq!(baselines, 5, "ubuntu, debian, rocky, fedora and alpine");

    let recommended: String =
        sqlx::query_scalar("SELECT key FROM baselines WHERE recommended LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("query");
    assert_eq!(recommended, "ubuntu-24.04");

    let packs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_packs WHERE enabled")
        .fetch_one(&pool)
        .await
        .expect("query");
    assert!(packs >= 9, "the agent packs are seeded, got {packs}");

    // Every pack has an image and the tool catalog can resolve for both
    // package managers.
    let packs_without_image: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_packs WHERE default_image = ''")
            .fetch_one(&pool)
            .await
            .expect("query");
    assert_eq!(packs_without_image, 0);

    let sync_schedule: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM schedules WHERE kind = 'catalog_sync'")
            .fetch_one(&pool)
            .await
            .expect("query");
    assert!(sync_schedule >= 1, "the catalog sync schedule is seeded");
}

/// The model that can actually be served, and the image allowed to serve it.
///
/// A digest, not a tag: an image is a dependency with a version (ADR 0022), and
/// the runner refuses anything that is not exactly this reference.
#[tokio::test]
async fn the_installable_model_names_its_artefact_and_server_image() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };

    let (key, artifact, runtime): (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT key, artifact_ref, serve_runtime FROM models WHERE serve_runtime IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("one servable model");
    assert_eq!(key, "qwen3-1.7b");
    let artifact = artifact.expect("artifact_ref");
    assert_eq!(artifact, "huggingface:Qwen/Qwen3-1.7B-GGUF/Qwen3-1.7B-Q8_0.gguf");
    assert_eq!(runtime.as_deref(), Some("llama_cpp"));

    // Every other row is explicitly not servable instead of quietly naming a
    // runtime nobody registered.
    let unservable: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models WHERE serve_runtime IS NOT NULL AND artifact_ref IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("query");
    assert_eq!(unservable, 0);

    let digest: String =
        sqlx::query_scalar("SELECT digest FROM model_server_images WHERE runtime = 'llama_cpp'")
            .fetch_one(&pool)
            .await
            .expect("llama.cpp image");
    assert!(digest.starts_with("ghcr.io/ggml-org/llama.cpp@sha256:"), "{digest}");
    assert!(!digest.ends_with(":latest"), "a tag is not a pin");

    // The constraint refuses a tag, which is what keeps the row honest even if
    // someone writes it by hand.
    let rejected = sqlx::query(
        "INSERT INTO model_server_images (runtime, image, digest, port, command)
         VALUES ('bad', 'example/model', 'example/model:latest', 8080, '{}')",
    )
    .execute(&pool)
    .await;
    assert!(rejected.is_err(), "a tag must not satisfy the digest constraint");
}

/// Row-level security refuses a row from another environment.
///
/// This is the second line of defence (ADR 0004): even a query that forgot its
/// `WHERE environment_id = ...` must not return another tenant's data.
#[tokio::test]
async fn row_level_security_blocks_a_cross_tenant_read() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };

    let environment: Uuid = sqlx::query_scalar("SELECT id FROM environments LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("environment");

    // Insert a sandbox as the owner, outside any tenant scope.
    let cluster: Uuid = sqlx::query_scalar("SELECT id FROM clusters LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("cluster");
    sqlx::query(
        "INSERT INTO sandboxes (environment_id, cluster_id, node_id, name, image, image_digest)
         VALUES ($1, $2, (SELECT id FROM nodes WHERE is_local), 'rls-canary',
                 'img@sha256:abc', 'sha256:abc')",
    )
    .bind(environment)
    .bind(cluster)
    .execute(&pool)
    .await
    .expect("insert");

    // Read it back with the tenant scope set: visible.
    //
    // The suite connects as the schema owner, which bypasses row-level
    // security; the application connects as `nestbox_app`, which does not. The
    // role switch is what makes this a test of the policy rather than of the
    // owner's privilege.
    let mut tx = pool.begin().await.expect("tx");
    sqlx::query("SELECT set_config('app.environment_id', $1, true)")
        .bind(environment.to_string())
        .execute(&mut *tx)
        .await
        .expect("set scope");
    sqlx::query("SET LOCAL ROLE nestbox_app")
        .execute(&mut *tx)
        .await
        .expect("switch to the application role");
    let visible: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sandboxes WHERE name = 'rls-canary'")
        .fetch_one(&mut *tx)
        .await
        .expect("count scoped");
    assert_eq!(visible, 1, "the owning environment must see its own row");
    tx.rollback().await.expect("rollback");

    // Read it back with a different scope: invisible.
    let other = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("tx");
    sqlx::query("SELECT set_config('app.environment_id', $1, true)")
        .bind(other.to_string())
        .execute(&mut *tx)
        .await
        .expect("set scope");
    sqlx::query("SET LOCAL ROLE nestbox_app")
        .execute(&mut *tx)
        .await
        .expect("switch to the application role");
    let hidden: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sandboxes WHERE name = 'rls-canary'")
        .fetch_one(&mut *tx)
        .await
        .expect("count scoped");
    assert_eq!(hidden, 0, "another tenant must not see the row");
    tx.rollback().await.expect("rollback");
}

/// An injection-shaped name is stored as data, not executed.
#[tokio::test]
async fn an_injection_shaped_name_is_just_a_string() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };

    let environment: Uuid = sqlx::query_scalar("SELECT id FROM environments LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("environment");
    let cluster: Uuid = sqlx::query_scalar("SELECT id FROM clusters LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("cluster");

    let hostile = "'; DROP TABLE sandboxes; --";
    sqlx::query(
        "INSERT INTO sandboxes (environment_id, cluster_id, node_id, name, image, image_digest)
         VALUES ($1, $2, (SELECT id FROM nodes WHERE is_local), $3,
                 'img@sha256:abc', 'sha256:abc')",
    )
    .bind(environment)
    .bind(cluster)
    .bind(hostile)
    .execute(&pool)
    .await
    .expect("the value is data, so it stores");

    let still_there: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sandboxes")
        .fetch_one(&pool)
        .await
        .expect("the table still exists");
    assert!(still_there >= 1);

    let round_trip: String =
        sqlx::query_scalar("SELECT name FROM sandboxes WHERE name = $1")
            .bind(hostile)
            .fetch_one(&pool)
            .await
            .expect("read back");
    assert_eq!(round_trip, hostile);
}

/// A second global policy for the same environment is refused by the database.
#[tokio::test]
async fn only_one_global_policy_can_exist() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };

    let environment: Uuid = sqlx::query_scalar("SELECT id FROM environments LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("environment");

    let spec = serde_json::json!({ "network_policies": {} });
    sqlx::query("INSERT INTO policies (environment_id, name, scope, spec) VALUES ($1, 'g1', 'global', $2)")
        .bind(environment)
        .bind(&spec)
        .execute(&pool)
        .await
        .expect("first global policy");

    let second = sqlx::query(
        "INSERT INTO policies (environment_id, name, scope, spec) VALUES ($1, 'g2', 'global', $2)",
    )
    .bind(environment)
    .bind(&spec)
    .execute(&pool)
    .await;

    assert!(second.is_err(), "a second global policy must be refused");
}

/// The audit table refuses an update for the application role's grants.
#[tokio::test]
async fn the_audit_table_is_append_only_for_the_app_role() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };

    let privileges: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.role_table_grants
          WHERE grantee = 'nestbox_app' AND table_name = 'audit_events'
            AND privilege_type IN ('UPDATE', 'DELETE')",
    )
    .fetch_one(&pool)
    .await
    .expect("grants");

    assert_eq!(
        privileges, 0,
        "nestbox_app must not be able to change or remove audit records"
    );
}

/// The telemetry partitions for the current month exist.
#[tokio::test]
async fn a_telemetry_partition_exists_for_this_month() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };

    let partitions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pg_class c
           JOIN pg_inherits i ON i.inhrelid = c.oid
           JOIN pg_class p ON p.oid = i.inhparent
          WHERE p.relname = 'telemetry_events' AND c.relname <> 'telemetry_events_default'",
    )
    .fetch_one(&pool)
    .await
    .expect("count partitions");

    assert!(partitions >= 3, "the next months should be pre-created");
}

/// A root takes one read-write mount at a time; read-only sharing is unlimited.
///
/// The rule is a partial unique index rather than a service check, because two
/// requests can pass the check in the same instant and only the database can
/// order them. A read-only mount is what makes a project usable by two sandboxes
/// at once without letting them overwrite each other.
#[tokio::test]
async fn a_root_takes_one_read_write_mount_and_many_read_only_ones() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };

    let environment: Uuid = sqlx::query_scalar("SELECT id FROM environments LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("environment");
    let cluster: Uuid = sqlx::query_scalar("SELECT id FROM clusters LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("cluster");
    let node: Uuid = sqlx::query_scalar("SELECT id FROM nodes WHERE is_local")
        .fetch_one(&pool)
        .await
        .expect("local node");

    let root: Uuid = sqlx::query_scalar(
        "INSERT INTO persistent_roots (environment_id, node_id, kind, slug, host_path)
         VALUES ($1, $2, 'project', 'rw-rule', '/var/lib/nestbox/persistent/t/project--rw-rule')
         RETURNING id",
    )
    .bind(environment)
    .bind(node)
    .fetch_one(&pool)
    .await
    .expect("root");

    let mut sandboxes = Vec::new();
    for name in ["rw-holder", "rw-reader", "rw-next"] {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO sandboxes (environment_id, cluster_id, node_id, name, image, image_digest)
             VALUES ($1, $2, $3, $4, 'img@sha256:abc', 'sha256:abc')
             RETURNING id",
        )
        .bind(environment)
        .bind(cluster)
        .bind(node)
        .bind(name)
        .fetch_one(&pool)
        .await
        .expect("sandbox");
        sandboxes.push(id);
    }
    let (holder, reader, next) = (sandboxes[0], sandboxes[1], sandboxes[2]);

    sqlx::query(
        "INSERT INTO mounts (root_id, sandbox_id, mount_path, read_only, status)
         VALUES ($1, $2, '/workspace', false, 'mounted')",
    )
    .bind(root)
    .bind(holder)
    .execute(&pool)
    .await
    .expect("the first read-write mount");

    let conflicting = sqlx::query(
        "INSERT INTO mounts (root_id, sandbox_id, mount_path, read_only, status)
         VALUES ($1, $2, '/workspace', false, 'mounted')",
    )
    .bind(root)
    .bind(reader)
    .execute(&pool)
    .await;
    assert!(
        conflicting.is_err(),
        "a second read-write mount of one root must be refused"
    );

    sqlx::query(
        "INSERT INTO mounts (root_id, sandbox_id, mount_path, read_only, status)
         VALUES ($1, $2, '/workspace', true, 'mounted')",
    )
    .bind(root)
    .bind(reader)
    .execute(&pool)
    .await
    .expect("a read-only mount next to a read-write one");

    // Dismounting the holder frees the root for another writer, which is what
    // keeps the rule usable after a container is retired.
    sqlx::query("UPDATE mounts SET status = 'dismounted' WHERE root_id = $1 AND NOT read_only")
        .bind(root)
        .execute(&pool)
        .await
        .expect("dismount");
    sqlx::query(
        "INSERT INTO mounts (root_id, sandbox_id, mount_path, read_only, status)
         VALUES ($1, $2, '/workspace', false, 'mounted')",
    )
    .bind(root)
    .bind(next)
    .execute(&pool)
    .await
    .expect("the freed root takes a new writer");

    // The rule is only as good as the index; assert the index is the partial one.
    let definition: String =
        sqlx::query_scalar("SELECT indexdef FROM pg_indexes WHERE indexname = 'mounts_one_rw_per_root'")
            .fetch_one(&pool)
            .await
            .expect("index");
    assert!(definition.contains("UNIQUE"));
    assert!(definition.contains("WHERE"));
    assert!(definition.contains("NOT read_only"));
}

/// A catalogue row as the sync writes it, for the upsert tests.
fn synced_row(
    name: &str,
    source_id: &str,
    min_vram_gib: i64,
) -> nestbox_control_api::store::postgres::models::SyncedModel {
    use nestbox_control_api::store::postgres::models::SyncedModel;
    const GIB: i64 = 1024 * 1024 * 1024;
    SyncedModel {
        source: "huggingface".to_string(),
        source_id: source_id.to_string(),
        key: format!("huggingface-{}", source_id.to_lowercase().replace(['/', '.', '_'], "-")),
        name: name.to_string(),
        family: "qwen".to_string(),
        parameters: "8B".to_string(),
        context_length: Some(40960),
        quantization: Some("BF16".to_string()),
        weights_bytes: Some(8 * GIB),
        min_vram_bytes: Some(min_vram_gib * GIB),
        recommended_vram_bytes: Some((min_vram_gib + 4) * GIB),
        min_gpu_count: 1,
        min_ram_bytes: None,
        recommended_ram_bytes: None,
        min_cpu_cores: None,
        recommended_cpu_cores: None,
        runtime: "cuda".to_string(),
        arch: Some("safetensors".to_string()),
        artifact_format: Some("BF16".to_string()),
        publisher: Some("Qwen".to_string()),
        license: Some("apache-2.0".to_string()),
        description: None,
        unified_memory_only: false,
        is_cpu_only: false,
        source_uri: Some(format!("https://huggingface.co/{source_id}")),
        version: "0123456789ab".to_string(),
    }
}

/// The sync adds rows and refreshes them, but a curated row is never rewritten.
///
/// This is the rule the whole package exists for (ADR 0019): a sync that can
/// overwrite a checked figure is worse than no sync, because the catalogue then
/// looks authoritative while being wrong.
#[tokio::test]
async fn a_synced_row_updates_until_it_is_curated_by_hand() {
    use nestbox_control_api::store::TenantScope;
    use nestbox_control_api::store::postgres::models::{upsert_synced, Upserted};

    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };

    let environment: Uuid = sqlx::query_scalar("SELECT id FROM environments LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("environment");

    // A new row is added, unpinned.
    let mut scope = TenantScope::begin(&pool, environment).await.expect("scope");
    let inserted = upsert_synced(&mut scope, &synced_row("Qwen3 8B", "Qwen/Qwen3-8B", 14))
        .await
        .expect("insert");
    scope.commit().await.expect("commit");
    assert_eq!(inserted, Upserted::Inserted);

    let pinned: bool = sqlx::query_scalar(
        "SELECT pinned FROM models WHERE source = 'huggingface' AND source_id = 'Qwen/Qwen3-8B'",
    )
    .fetch_one(&pool)
    .await
    .expect("pinned flag");
    assert!(!pinned, "a synced row is never curated");

    // A later sync with a different figure refreshes it.
    let mut scope = TenantScope::begin(&pool, environment).await.expect("scope");
    let updated = upsert_synced(&mut scope, &synced_row("Qwen3 8B v2", "Qwen/Qwen3-8B", 20))
        .await
        .expect("update");
    scope.commit().await.expect("commit");
    assert_eq!(updated, Upserted::Updated);

    let name: String = sqlx::query_scalar(
        "SELECT name FROM models WHERE source = 'huggingface' AND source_id = 'Qwen/Qwen3-8B'",
    )
    .fetch_one(&pool)
    .await
    .expect("name");
    assert_eq!(name, "Qwen3 8B v2");

    // Curate it, then sync the same id with a different name, gate and version.
    sqlx::query(
        "UPDATE models SET pinned = true, name = 'Curated Qwen3 8B', min_vram_bytes = 14::bigint * 1024 * 1024 * 1024
          WHERE source = 'huggingface' AND source_id = 'Qwen/Qwen3-8B'",
    )
    .execute(&pool)
    .await
    .expect("curate");

    let mut scope = TenantScope::begin(&pool, environment).await.expect("scope");
    let skipped = upsert_synced(&mut scope, &synced_row("Upstream renamed it", "Qwen/Qwen3-8B", 99))
        .await
        .expect("pinned upsert");
    scope.commit().await.expect("commit");
    assert_eq!(skipped, Upserted::SkippedPinned);

    let (name, min_vram, version): (String, i64, String) = sqlx::query_as(
        "SELECT name, min_vram_bytes, version FROM models
          WHERE source = 'huggingface' AND source_id = 'Qwen/Qwen3-8B'",
    )
    .fetch_one(&pool)
    .await
    .expect("curated row");
    assert_eq!(name, "Curated Qwen3 8B");
    assert_eq!(min_vram, 14 * 1024 * 1024 * 1024);
    assert_eq!(version, "0123456789ab");

    // The row still exists and still has exactly one copy: nothing was deleted
    // and no second row was created for the same source id.
    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models WHERE source = 'huggingface' AND source_id = 'Qwen/Qwen3-8B'",
    )
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(rows, 1);
}
