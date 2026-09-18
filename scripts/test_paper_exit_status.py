import sqlite3
import unittest

from paper_exit_status import pending_exits


class PendingExitTests(unittest.TestCase):
    def test_retries_are_collapsed_and_later_fill_resolves_order(self):
        with sqlite3.connect(":memory:") as connection:
            connection.execute("""CREATE TABLE order_audit (
                id INTEGER PRIMARY KEY, business_order_id TEXT, source TEXT,
                side TEXT, code TEXT, quantity INTEGER, requested_price REAL,
                outcome TEXT, failure_reason TEXT, created_at TEXT)""")
            for order, outcome, timestamp in [
                ("pending", "Rejected", "2026-09-07 16:30:00"),
                ("pending", "Rejected", "2026-09-08 02:00:00"),
                ("resolved", "Rejected", "2026-09-08 01:30:00"),
                ("resolved", "Filled", "2026-09-08 02:00:00"),
                ("yesterday", "Rejected", "2026-09-07 15:59:59"),
            ]:
                connection.execute("""INSERT INTO order_audit
                    (business_order_id, source, side, code, quantity, requested_price,
                     outcome, failure_reason, created_at)
                    VALUES (?, 'PaperTrade', 'sell', 'TEST_CODE_000001', 100, 10, ?, '确认待处理', ?)""",
                    (order, outcome, timestamp))
            rows = pending_exits(connection, "2026-09-08")
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["business_order_id"], "pending")
        self.assertEqual(rows[0]["attempts"], 2)
        self.assertEqual(rows[0]["last_attempt_shanghai"], "2026-09-08 10:00:00")


if __name__ == "__main__":
    unittest.main()
