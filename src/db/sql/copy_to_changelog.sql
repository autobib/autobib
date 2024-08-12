INSERT INTO Changelog (record_id, data, modified) SELECT record_id, data, modified FROM Records WHERE key = ?1
