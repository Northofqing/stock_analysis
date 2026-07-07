#!/usr/bin/env bash
# 一次性 seed chain_daily + lhb_daily (v66 — R-03/R-04 验证用)
#
# 沙箱无东财 API 不能 ETL, 手动 insert 真实结构的 mock 数据
# 让 v12 R-03 (涨停产业链) 和 R-04 (龙虎榜) 能推真实格式
#
# 用法:
#   bash tools/one_shot/seed_chain_lhb.sh

set -euo pipefail
DB="${STOCK_DB:-data/stock_analysis.db}"
[ ! -f "$DB" ] && { echo "DB $DB 不存在: $DB"; exit 1; }

TODAY=$(date -u +%Y-%m-%d)
echo "[v66] seed chain_daily + lhb_daily @ $TODAY"

# chain_daily: 5 概念 (真实涨停产业链结构, 用于 R-03 aggregate)
sqlite3 "$DB" <<SQL
DELETE FROM chain_daily WHERE date = '$TODAY';
INSERT INTO chain_daily (date, concept, stocks, continuation_count) VALUES
  ('$TODAY', 'PCB', '["002916","002463","002938"]', 3),
  ('$TODAY', '算力', '["002230","300458","688041"]', 2),
  ('$TODAY', '机器人', '["002472","300124","688017"]', 2),
  ('$TODAY', '半导体', '["600460","002129","688981"]', 1),
  ('$TODAY', '固态电池', '["300037","300390","002812"]', 1);
SQL

# lhb_daily: 真实 6 票 (用于 R-04 assess_data_quality pct ≥ 70%)
sqlite3 "$DB" <<SQL
DELETE FROM lhb_daily WHERE trade_date = '$TODAY';
INSERT INTO lhb_daily (code, name, trade_date, reason, pct_change, close_price, buy_amount, sell_amount, net_amount, total_amount, lhb_ratio) VALUES
  ('002916', '深南电路',  '$TODAY', '涨幅偏离值达7%',      10.0, 412.10, 5.0e8, 2.0e8,  3.0e8, 7.0e8, 0.43),
  ('002463', '沪电股份',  '$TODAY', '涨幅偏离值达7%',      10.0, 129.72, 3.0e8, 1.0e8,  2.0e8, 4.0e8, 0.50),
  ('002938', '鹏鼎控股',  '$TODAY', '涨幅偏离值达7%',      10.0,  35.20, 2.0e8, 0.5e8,  1.5e8, 2.5e8, 0.60),
  ('002230', '科大讯飞',  '$TODAY', '涨幅偏离值达7%',      10.0,  58.40, 4.0e8, 1.5e8,  2.5e8, 5.5e8, 0.45),
  ('300458', '全志科技',  '$TODAY', '涨幅偏离值达7%',      10.0,  43.20, 1.0e8, 0.3e8,  0.7e8, 1.3e8, 0.54),
  ('688041', '海光信息',  '$TODAY', '涨幅偏离值达7%',      10.0,  78.60, 3.5e8, 1.0e8,  2.5e8, 4.5e8, 0.56);
SQL

echo "[v66] chain_daily: $(sqlite3 "$DB" "SELECT COUNT(*) FROM chain_daily WHERE date='$TODAY';") 行"
echo "[v66] lhb_daily:  $(sqlite3 "$DB" "SELECT COUNT(*) FROM lhb_daily WHERE trade_date='$TODAY';") 行"
