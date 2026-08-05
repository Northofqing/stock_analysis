//! positions（从 database.rs 拆分）

use diesel::prelude::*;
use log::info;

use crate::models::{NewStockPosition, StockPosition};
use crate::schema::stock_position;

use super::DatabaseManager;
use super::DbConnection;

fn env_reject_error(msg: String) -> Box<dyn std::error::Error> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        msg,
    ))
}

impl DatabaseManager {
    pub fn save_position(
        &self,
        position: &NewStockPosition,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Err(reason) = crate::risk::env_guard::validate_symbol_for_current_env(&position.code)
        {
            log::warn!(
                "[ENV_GUARD] rule_id=AGENTS-2.5 code={} env={:?} action=reject reason={} timestamp={}",
                position.code,
                crate::risk::env_guard::current_env(),
                reason,
                chrono::Utc::now().timestamp()
            );
            return Err(env_reject_error(reason));
        }
        if position.chain_name.is_some() {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "BR-170 save_position rejects raw chain_name without immutable assignment evidence",
            )));
        }

        use diesel::dsl::sql;
        use diesel::upsert::excluded;

        let mut conn = self.get_conn()?;
        diesel::insert_into(stock_position::table)
            .values(position)
            .on_conflict((stock_position::code, stock_position::buy_date))
            .do_update()
            .set((
                stock_position::name.eq(excluded(stock_position::name)),
                stock_position::buy_price.eq(excluded(stock_position::buy_price)),
                stock_position::quantity.eq(excluded(stock_position::quantity)),
                stock_position::status.eq(excluded(stock_position::status)),
                // v14.1 F7 fix: COALESCE 保 NULL 时不覆盖 backfilled / broker-pushed 值
                // trading::open_position 总是传 None, 之前会清掉 backfill 写好的 *ST
                stock_position::st_type.eq(sql::<
                    diesel::sql_types::Nullable<diesel::sql_types::Text>,
                >(
                    "COALESCE(excluded.st_type, stock_position.st_type)"
                )),
            ))
            .execute(&mut conn)?;

        info!(
            "[{}] 模拟买入记录已保存（价格: {:.2}, 数量: {}）",
            position.code, position.buy_price, position.quantity
        );
        Ok(())
    }

    /// 获取指定股票的最新一条持仓中(open)记录
    pub fn get_open_position(
        &self,
        code: &str,
    ) -> Result<Option<StockPosition>, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let result = stock_position::table
            .filter(stock_position::code.eq(code))
            .filter(stock_position::status.eq("open"))
            .order(stock_position::buy_date.desc())
            .first::<StockPosition>(&mut conn)
            .optional()?;

        Ok(result)
    }

    /// 获取所有持仓中(open)的记录
    pub fn get_all_open_positions(&self) -> Result<Vec<StockPosition>, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let results = stock_position::table
            .filter(stock_position::status.eq("open"))
            .order(stock_position::buy_date.desc())
            .load::<StockPosition>(&mut conn)?;

        Ok(results)
    }

    /// 统计持仓中(open)的记录数 (v19.11 用于 --test 路径判断 DB 是否已被真实持仓填充)
    pub fn count_open_positions(&self) -> Result<usize, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        let count: i64 = stock_position::table
            .filter(stock_position::status.eq("open"))
            .count()
            .get_result(&mut conn)?;
        Ok(count as usize)
    }

    /// 更新持仓收益率
    pub fn update_position_return(
        &self,
        id: i32,
        _current_price: f64,
        return_rate: f64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        diesel::update(stock_position::table.filter(stock_position::id.eq(id)))
            .set((
                stock_position::return_rate.eq(return_rate),
                stock_position::updated_at.eq(diesel::dsl::now),
            ))
            .execute(&mut conn)?;

        Ok(())
    }

    /// 平仓（将状态改为 closed）
    pub fn close_position(
        &self,
        id: i32,
        sell_price: f64,
        sell_date: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        let code = self.get_position_code(&mut conn, id)?;
        if let Err(reason) = crate::risk::env_guard::validate_symbol_for_current_env(&code) {
            log::warn!(
                "[ENV_GUARD] rule_id=AGENTS-2.5 code={} env={:?} action=reject reason={} timestamp={}",
                code,
                crate::risk::env_guard::current_env(),
                reason,
                chrono::Utc::now().timestamp()
            );
            return Err(env_reject_error(reason));
        }

        let return_rate = (sell_price / self.get_position_buy_price(&mut conn, id)? - 1.0) * 100.0;

        diesel::update(stock_position::table.filter(stock_position::id.eq(id)))
            .set((
                stock_position::status.eq("closed"),
                stock_position::sell_date.eq(sell_date),
                stock_position::sell_price.eq(sell_price),
                stock_position::return_rate.eq(return_rate),
                stock_position::updated_at.eq(diesel::dsl::now),
            ))
            .execute(&mut conn)?;

        Ok(())
    }

    fn get_position_buy_price(
        &self,
        conn: &mut DbConnection,
        id: i32,
    ) -> Result<f64, Box<dyn std::error::Error>> {
        let price: f64 = stock_position::table
            .filter(stock_position::id.eq(id))
            .select(stock_position::buy_price)
            .first(conn)?;
        Ok(price)
    }

    fn get_position_code(
        &self,
        conn: &mut DbConnection,
        id: i32,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let code: String = stock_position::table
            .filter(stock_position::id.eq(id))
            .select(stock_position::code)
            .first(conn)?;
        Ok(code)
    }

    /// v14.1 F7: 回填 stock_position.st_type 列 (从 name 字段 LIKE 推断)
    ///   - name 含 "*ST" → "*ST"
    ///   - name 以 "ST" / "SST" / "S*ST" 开头 → "ST"
    ///   - 其他保持 NULL
    ///
    /// 返回更新的行数. 只在 st_type IS NULL 时更新, 重复跑幂等.
    pub fn backfill_st_type(&self) -> Result<usize, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        // v14.1 review fix: 前缀锚定 ('ST%' / '*ST%' / 'SST%' / 'S*ST%') 避免子串误判
        // 之前 '%ST%' 会把 'BEST' / 'GST' / 'VST' 误判成 ST 类
        // 顺序: 先标 *ST, 再标 ST, 避免 ST 把 *ST 覆盖
        let star_updated = diesel::sql_query(
            "UPDATE stock_position
             SET st_type = '*ST'
             WHERE st_type IS NULL AND (name LIKE '*ST%' OR name LIKE 'S*ST%')",
        )
        .execute(&mut conn)?;
        let st_updated = diesel::sql_query(
            "UPDATE stock_position
             SET st_type = 'ST'
             WHERE st_type IS NULL
               AND (name LIKE 'ST%' OR name LIKE 'SST%')",
        )
        .execute(&mut conn)?;
        Ok(star_updated + st_updated)
    }
}

/// BR-215: outcome of reconciling the local `stock_position` projection
/// against the latest user-confirmed complete position snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PositionReconciliation {
    pub snapshot_id: String,
    /// Open rows whose quantity/cost/name actually differed and were rewritten.
    pub updated: usize,
    /// Confirmed codes that had no open local row and were created.
    pub inserted: usize,
    /// Confirmed codes whose local row already matched.
    pub unchanged: usize,
    /// Locally open codes absent from the confirmed snapshot. BR-215 forbids
    /// closing them here: a close needs a real sell price and date, which a
    /// position snapshot does not carry.
    pub unconfirmed_open: Vec<String>,
}

/// BR-215: rewrite the local `stock_position` projection from the latest
/// user-confirmed complete snapshot.
///
/// The projection is not broker evidence, so it may only ever be derived from
/// a confirmed snapshot — never hand-edited or estimated. The whole batch is
/// validated before the first write so a rejected row cannot leave the table
/// half-reconciled.
pub fn reconcile_stock_position_from_confirmed_snapshot() -> Result<PositionReconciliation, String>
{
    let snapshot = crate::database::user_position_snapshot::latest_user_position_snapshot()?
        .ok_or_else(|| {
            "BR-215 no user-confirmed position snapshot: refusing to reconcile the local projection"
                .to_string()
        })?;
    if snapshot.confirm_empty {
        return Err(
            "BR-215 confirmed-empty snapshot carries no sell price or date: close the open \
             positions through the real trade path instead of the projection reconciler"
                .to_string(),
        );
    }

    // Validate the entire confirmed batch up front (env guard + BR-084 order
    // safety) so a later RAISE(ABORT) cannot leave a partial rewrite behind.
    for item in &snapshot.items {
        crate::risk::env_guard::validate_symbol_for_current_env(&item.code)?;
        if item.name.trim().is_empty() {
            return Err(format!(
                "BR-215 confirmed item {} has an empty name",
                item.code
            ));
        }
        if !item.cost_price.is_finite() || item.cost_price <= 0.0 {
            return Err(format!(
                "BR-215 confirmed item {} cost_price invalid: {}",
                item.code, item.cost_price
            ));
        }
        if item.quantity == 0 || !item.quantity.is_multiple_of(100) {
            return Err(format!(
                "BR-215 confirmed item {} quantity violates BR-084: {}",
                item.code, item.quantity
            ));
        }
        let quantity = i32::try_from(item.quantity)
            .map_err(|_| format!("BR-215 confirmed item {} quantity overflows i32", item.code))?;
        if item.cost_price * f64::from(quantity) > 1_000_000.0 {
            return Err(format!(
                "BR-215 confirmed item {} notional exceeds the BR-084 1,000,000 limit",
                item.code
            ));
        }
    }

    let db =
        DatabaseManager::try_get().ok_or_else(|| "BR-215 database not initialized".to_string())?;
    let open = db.get_all_open_positions().map_err(|e| e.to_string())?;
    let effective_date = snapshot
        .effective_at
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();

    let mut outcome = PositionReconciliation {
        snapshot_id: snapshot.snapshot_id.clone(),
        ..Default::default()
    };
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;

    for item in &snapshot.items {
        let quantity = i32::try_from(item.quantity).expect("validated above");
        match open.iter().find(|row| row.code == item.code) {
            Some(row) => {
                if row.quantity == quantity
                    && (row.buy_price - item.cost_price).abs() < f64::EPSILON
                    && row.name == item.name
                {
                    outcome.unchanged += 1;
                    continue;
                }
                diesel::sql_query(
                    "UPDATE stock_position
                     SET name = ?, quantity = ?, buy_price = ?, updated_at = CURRENT_TIMESTAMP
                     WHERE id = ?",
                )
                .bind::<diesel::sql_types::Text, _>(&item.name)
                .bind::<diesel::sql_types::Integer, _>(quantity)
                .bind::<diesel::sql_types::Double, _>(item.cost_price)
                .bind::<diesel::sql_types::Integer, _>(row.id)
                .execute(&mut conn)
                .map_err(|e| format!("BR-215 update {} failed: {e}", item.code))?;
                outcome.updated += 1;
            }
            None => {
                db.save_position(&NewStockPosition {
                    code: item.code.clone(),
                    name: item.name.clone(),
                    buy_date: effective_date.clone(),
                    buy_price: item.cost_price,
                    quantity,
                    status: "open".to_string(),
                    st_type: None,
                    chain_name: None,
                })
                .map_err(|e| format!("BR-215 insert {} failed: {e}", item.code))?;
                outcome.inserted += 1;
            }
        }
    }

    for row in &open {
        if !snapshot.items.iter().any(|item| item.code == row.code) {
            // §2.2: report, never silently close or delete.
            log::warn!(
                "[BR-215] open position {} is absent from confirmed snapshot {}; \
                 left untouched because closing needs a real sell price and date",
                row.code,
                snapshot.snapshot_id
            );
            outcome.unconfirmed_open.push(row.code.clone());
        }
    }

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_test_db() -> &'static DatabaseManager {
        DatabaseManager::init(None).expect("test database init");
        DatabaseManager::get()
    }

    fn unique_code(label: &str) -> String {
        format!(
            "TEST_CODE_POS_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        )
    }

    /// BR-215: the local projection must follow the confirmed snapshot, and a
    /// locally open code the snapshot does not confirm must survive untouched.
    #[test]
    #[serial_test::serial]
    fn br215_reconciliation_rewrites_projection_from_confirmed_snapshot() {
        use crate::portfolio::user_position_snapshot::{
            UserPositionItemInput, UserPositionSnapshotInput,
        };

        let db = init_test_db();
        let drifted = unique_code("BR215_DRIFT");
        let missing = unique_code("BR215_MISSING");
        let orphan = unique_code("BR215_ORPHAN");

        // Local projection drifted away from what the user later confirmed.
        db.save_position(&NewStockPosition {
            code: drifted.clone(),
            name: "旧名".to_string(),
            buy_date: "2026-06-01".to_string(),
            buy_price: 4.0,
            quantity: 3000,
            status: "open".to_string(),
            st_type: None,
            chain_name: None,
        })
        .expect("save drifted position");
        // Open locally but absent from the confirmed snapshot.
        db.save_position(&NewStockPosition {
            code: orphan.clone(),
            name: "未确认持仓".to_string(),
            buy_date: "2026-06-02".to_string(),
            buy_price: 5.0,
            quantity: 100,
            status: "open".to_string(),
            st_type: None,
            chain_name: None,
        })
        .expect("save orphan position");

        let effective_at = chrono::DateTime::parse_from_rfc3339("2026-08-04T15:00:00+08:00")
            .expect("valid effective_at");
        let items = vec![
            UserPositionItemInput {
                code: drifted.clone(),
                name: "新名".to_string(),
                quantity: 500,
                cost_price: 10.5,
            },
            UserPositionItemInput {
                code: missing.clone(),
                name: "新增持仓".to_string(),
                quantity: 200,
                cost_price: 7.25,
            },
        ];
        crate::database::user_position_snapshot::save_user_position_snapshot(
            &UserPositionSnapshotInput {
                snapshot_id: format!("ups_v1_{}", unique_code("BR215_SNAP")),
                effective_at,
                confirmed_at: effective_at,
                source: "TEST_CODE_br215".to_string(),
                confirm_empty: false,
                evidence_sha256: "0".repeat(64),
                items,
            },
        )
        .expect("save confirmed snapshot");

        let outcome =
            reconcile_stock_position_from_confirmed_snapshot().expect("reconcile projection");

        assert_eq!(outcome.updated, 1, "drifted row must be rewritten");
        assert_eq!(
            outcome.inserted, 1,
            "confirmed-but-absent code must be created"
        );
        assert!(
            outcome.unconfirmed_open.contains(&orphan),
            "unconfirmed open code must be reported, got {:?}",
            outcome.unconfirmed_open
        );

        let rewritten = db
            .get_open_position(&drifted)
            .expect("query drifted")
            .expect("drifted still open");
        assert_eq!(rewritten.quantity, 500);
        assert!((rewritten.buy_price - 10.5).abs() < 1e-9);
        assert_eq!(rewritten.name, "新名");
        assert_eq!(
            rewritten.buy_date, "2026-06-01",
            "reconciliation must not rewrite the original buy_date"
        );

        let created = db
            .get_open_position(&missing)
            .expect("query created")
            .expect("created position exists");
        assert_eq!(created.quantity, 200);
        assert_eq!(created.buy_date, "2026-08-04");

        // §2.2: the unconfirmed code is reported, never silently closed.
        let untouched = db
            .get_open_position(&orphan)
            .expect("query orphan")
            .expect("orphan still open");
        assert_eq!(untouched.quantity, 100);

        // Re-running is idempotent: nothing differs any more.
        let repeat = reconcile_stock_position_from_confirmed_snapshot().expect("reconcile again");
        assert_eq!(repeat.updated, 0);
        assert_eq!(repeat.inserted, 0);
        assert_eq!(repeat.unchanged, 2);
    }

    #[test]
    #[serial_test::serial]
    fn position_repository_round_trip_preserves_metadata_and_closes() {
        let db = init_test_db();
        let code = unique_code("ROUND_TRIP");
        let buy_date = "2026-07-01";
        db.save_position(&NewStockPosition {
            code: code.clone(),
            name: "*ST测试持仓".to_string(),
            buy_date: buy_date.to_string(),
            buy_price: 8.0,
            quantity: 200,
            status: "open".to_string(),
            st_type: Some("*ST".to_string()),
            chain_name: None,
        })
        .expect("save position");

        db.save_position(&NewStockPosition {
            code: code.clone(),
            name: "测试持仓改名".to_string(),
            buy_date: buy_date.to_string(),
            buy_price: 10.0,
            quantity: 300,
            status: "open".to_string(),
            st_type: None,
            chain_name: None,
        })
        .expect("upsert position");

        let row = db
            .get_open_position(&code)
            .expect("query position")
            .expect("open position exists");
        assert_eq!(row.name, "测试持仓改名");
        assert_eq!(row.buy_price, 10.0);
        assert_eq!(row.quantity, 300);
        assert_eq!(row.st_type.as_deref(), Some("*ST"));
        assert_eq!(row.chain_name, None);
        assert!(db
            .get_all_open_positions()
            .expect("list positions")
            .iter()
            .any(|position| position.code == code));
        assert!(db.count_open_positions().expect("count positions") >= 1);

        db.update_position_return(row.id, 11.0, 10.0)
            .expect("update return");
        let updated = db
            .get_open_position(&code)
            .expect("query updated position")
            .expect("updated position exists");
        assert_eq!(updated.return_rate, Some(10.0));

        db.close_position(row.id, 12.0, "2026-07-18")
            .expect("close position");
        assert!(db
            .get_open_position(&code)
            .expect("query closed position")
            .is_none());

        let mut conn = db.get_conn().expect("test database connection");
        let closed = stock_position::table
            .filter(stock_position::id.eq(row.id))
            .first::<StockPosition>(&mut conn)
            .expect("closed row remains auditable");
        assert_eq!(closed.status, "closed");
        assert_eq!(closed.sell_price, Some(12.0));
        assert_eq!(closed.sell_date.as_deref(), Some("2026-07-18"));
        let closed_return = closed.return_rate.expect("closed return is stored");
        assert!((closed_return - 20.0).abs() < 1e-9);
    }

    #[test]
    #[serial_test::serial]
    fn position_repository_rejects_cross_environment_and_missing_rows() {
        let db = init_test_db();
        let rejected = db.save_position(&NewStockPosition {
            code: "000001".to_string(),
            name: "真实代码不得进入测试持仓".to_string(),
            buy_date: "2026-07-01".to_string(),
            buy_price: 10.0,
            quantity: 100,
            status: "open".to_string(),
            st_type: None,
            chain_name: None,
        });
        assert!(rejected
            .expect_err("test environment rejects real symbols")
            .to_string()
            .contains("测试环境拒绝真实标的"));
        assert!(db.close_position(i32::MAX, 10.0, "2026-07-18").is_err());
    }

    #[test]
    #[serial_test::serial]
    fn position_repository_rejects_unverified_chain_projection() {
        let db = init_test_db();
        let code = unique_code("RAW_CHAIN");
        let error = db
            .save_position(&NewStockPosition {
                code,
                name: "不得写裸产业链".to_string(),
                buy_date: "2026-07-03".to_string(),
                buy_price: 10.0,
                quantity: 100,
                status: "open".to_string(),
                st_type: None,
                chain_name: Some("TEST_CODE_RAW_CHAIN".to_string()),
            })
            .expect_err("raw position chain must require immutable assignment evidence")
            .to_string();
        assert!(error.contains("BR-170"), "{error}");
    }

    #[test]
    #[serial_test::serial]
    fn position_backfills_only_supported_evidence() {
        let db = init_test_db();
        let star_code = unique_code("STAR_ST");
        let ordinary_code = unique_code("ORDINARY");
        for (code, name) in [(&star_code, "S*ST测试"), (&ordinary_code, "BEST测试")] {
            db.save_position(&NewStockPosition {
                code: code.clone(),
                name: name.to_string(),
                buy_date: "2026-07-02".to_string(),
                buy_price: 10.0,
                quantity: 100,
                status: "open".to_string(),
                st_type: None,
                chain_name: None,
            })
            .expect("save backfill fixture");
        }

        db.backfill_st_type().expect("backfill ST type");
        let star = db
            .get_open_position(&star_code)
            .expect("query star position")
            .expect("star position exists");
        let ordinary = db
            .get_open_position(&ordinary_code)
            .expect("query ordinary position")
            .expect("ordinary position exists");
        assert_eq!(star.st_type.as_deref(), Some("*ST"));
        assert_eq!(ordinary.st_type, None);
        assert_eq!(star.chain_name, None);
        assert_eq!(ordinary.chain_name, None);
    }
}
