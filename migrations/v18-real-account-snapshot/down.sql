-- BR-103 controlled rollback intentionally retains real account evidence and
-- its immutability/retention triggers. Older application versions ignore the
-- additive table; deleting it would violate the five-year audit requirement.
SELECT 1;
