CREATE VIEW ActiveRecords AS
SELECT canonical, modified, data as entry_data
FROM Records
WHERE
  rev in (SELECT record_rev FROM Keys)
  AND variant = 0;
