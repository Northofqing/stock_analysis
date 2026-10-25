-- PIT universe（在册口径）：每只票在 stock_daily 里的可交易区间。
-- 已退市票不在 TDX vipdir，故本表只覆盖在册代码；回测须声明「未含退市票」。
CREATE TABLE IF NOT EXISTS security_universe_pit (
  code TEXT PRIMARY KEY,
  exchange TEXT NOT NULL,
  first_date TEXT NOT NULL,
  last_date TEXT NOT NULL,
  trading_days INTEGER NOT NULL,
  last_gap_days INTEGER NOT NULL,
  status TEXT NOT NULL
);
DELETE FROM security_universe_pit;
INSERT INTO security_universe_pit
  (code, exchange, first_date, last_date, trading_days, last_gap_days, status)
SELECT
  code,
  CASE WHEN code LIKE '920%' OR code LIKE '8%' OR code LIKE '4%' THEN 'Beijing'
       WHEN code LIKE '6%' THEN 'Shanghai' ELSE 'Shenzhen' END,
  MIN(date),
  MAX(date),
  COUNT(*),
  CAST(julianday((SELECT MAX(date) FROM stock_daily)) - julianday(MAX(date)) AS INTEGER),
  CASE WHEN julianday((SELECT MAX(date) FROM stock_daily)) - julianday(MAX(date)) <= 7
       THEN 'current' ELSE 'stale' END
FROM stock_daily
GROUP BY code;
