use sqlx::SqlitePool;

use crate::DbError;
use crate::models::JavaVersionRow;

#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &SqlitePool,
    absolute_path: &str,
    major: u32,
    version: &str,
    vendor: &str,
    os_arch: &str,
    is_jdk: bool,
    probe_version: u32,
) -> Result<JavaVersionRow, DbError> {
    sqlx::query_as!(
        JavaVersionRow,
        r#"
		INSERT INTO java_versions (absolute_path, major, version, vendor, os_arch, is_jdk, probe_version)
		VALUES (?, ?, ?, ?, ?, ?, ?)
		ON CONFLICT(absolute_path) DO UPDATE SET
			major = excluded.major,
			version = excluded.version,
			vendor = excluded.vendor,
			os_arch = excluded.os_arch,
			is_jdk = excluded.is_jdk,
			probe_version = excluded.probe_version
		RETURNING absolute_path, major, version, vendor, os_arch, is_jdk as "is_jdk: bool", probe_version
		"#,
        absolute_path,
        i64::from(major),
        version,
        vendor,
        os_arch,
        is_jdk,
        i64::from(probe_version),
    )
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

pub async fn get_by_path(
    pool: &SqlitePool,
    absolute_path: &str,
) -> Result<Option<JavaVersionRow>, DbError> {
    let row = sqlx::query_as!(
        JavaVersionRow,
        r#"
        SELECT absolute_path, major, version, vendor, os_arch, is_jdk as "is_jdk: bool", probe_version
        FROM java_versions
        WHERE absolute_path = ?
        "#,
        absolute_path,
    )
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Picked from [`list_all`] since SQL would order `version` as text
pub async fn get_latest_by_major(
    pool: &SqlitePool,
    major: u32,
) -> Result<Option<JavaVersionRow>, DbError> {
    let major = i64::from(major);

    Ok(list_all(pool)
        .await?
        .into_iter()
        .find(|row| row.major == major))
}

pub async fn delete_by_path(pool: &SqlitePool, absolute_path: &str) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;

    sqlx::query!(
        r#"
        UPDATE setting_profiles SET java_path = NULL WHERE java_path = ?
        "#,
        absolute_path,
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        r#"
        DELETE FROM java_versions WHERE absolute_path = ?
        "#,
        absolute_path,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Newest first by major then by version compared numerically
pub async fn list_all(pool: &SqlitePool) -> Result<Vec<JavaVersionRow>, DbError> {
    let mut rows = sqlx::query_as!(
        JavaVersionRow,
        r#"
        SELECT absolute_path, major, version, vendor, os_arch, is_jdk as "is_jdk: bool", probe_version
        FROM java_versions
        ORDER BY major DESC, version DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    // SQL compares `version` as text which puts 21.0.9 above 21.0.10
    rows.sort_by(|a, b| {
        b.major
            .cmp(&a.major)
            .then_with(|| compare_versions(&b.version, &a.version))
    });

    Ok(rows)
}

/// Digit runs compare by value and everything else byte by byte so
/// `21.0.10` > `21.0.9` and `1.8.0_392` > `1.8.0_45`
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    fn split_digits(s: &[u8]) -> (&[u8], &[u8]) {
        let end = s.iter().position(|c| !c.is_ascii_digit()).unwrap_or(s.len());
        s.split_at(end)
    }

    fn trim_zeros(digits: &[u8]) -> &[u8] {
        let start = digits.iter().position(|&c| c != b'0').unwrap_or(digits.len());
        &digits[start..]
    }

    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    loop {
        match (a.first(), b.first()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (run_a, rest_a) = split_digits(a);
                let (run_b, rest_b) = split_digits(b);
                let (run_a, run_b) = (trim_zeros(run_a), trim_zeros(run_b));

                let order = run_a.len().cmp(&run_b.len()).then_with(|| run_a.cmp(run_b));
                if order != Ordering::Equal {
                    return order;
                }
                a = rest_a;
                b = rest_b;
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(y);
                }
                a = &a[1..];
                b = &b[1..];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        use std::cmp::Ordering;

        assert_eq!(compare_versions("21.0.10", "21.0.9"), Ordering::Greater);
        assert_eq!(compare_versions("1.8.0_392", "1.8.0_45"), Ordering::Greater);
        assert_eq!(compare_versions("17.0.2", "17.0.2"), Ordering::Equal);
        assert_eq!(compare_versions("21", "21.0.1"), Ordering::Less);
        assert_eq!(compare_versions("21.0.01", "21.0.1"), Ordering::Equal);
    }

    #[tokio::test]
    async fn listing_orders_versions_numerically() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::migrate!().run(&pool).await.expect("migrations run");

        for (path, major, version) in [
            ("/jdk21-9/bin/java", 21, "21.0.9"),
            ("/jdk21-10/bin/java", 21, "21.0.10"),
            ("/jdk17/bin/java", 17, "17.0.12"),
        ] {
            insert(&pool, path, major, version, "Zulu", "x64", true, 1)
                .await
                .expect("insert java");
        }

        let versions: Vec<String> = list_all(&pool)
            .await
            .expect("list")
            .into_iter()
            .map(|row| row.version)
            .collect();
        assert_eq!(versions, ["21.0.10", "21.0.9", "17.0.12"]);

        let latest = get_latest_by_major(&pool, 21)
            .await
            .expect("latest")
            .expect("a 21 runtime");
        assert_eq!(latest.version, "21.0.10");
    }

    #[tokio::test]
    async fn delete_clears_profile_reference() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::migrate!().run(&pool).await.expect("migrations run");

        insert(&pool, "/jdk/bin/java", 21, "21.0.1", "Zulu", "x64", true, 1)
            .await
            .expect("insert java");
        sqlx::query("INSERT INTO setting_profiles (name, java_path) VALUES ('p', '/jdk/bin/java')")
            .execute(&pool)
            .await
            .expect("insert profile");

        delete_by_path(&pool, "/jdk/bin/java")
            .await
            .expect("delete despite profile reference");

        let java_path: Option<String> =
            sqlx::query_scalar("SELECT java_path FROM setting_profiles WHERE name = 'p'")
                .fetch_one(&pool)
                .await
                .expect("profile kept");
        assert_eq!(java_path, None);
        assert!(get_by_path(&pool, "/jdk/bin/java").await.unwrap().is_none());
    }
}
