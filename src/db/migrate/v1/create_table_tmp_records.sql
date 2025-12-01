CREATE TABLE "tmp_Records" (
  "key" INTEGER PRIMARY KEY,
  "record_id" TEXT NOT NULL,
  "data" BLOB NOT NULL,
  "modified" TEXT NOT NULL,
  "variant" INTEGER NOT NULL DEFAULT 0,
  "parent_key" INTEGER REFERENCES "Records"(key)
    ON UPDATE RESTRICT
    ON DELETE SET NULL
) STRICT
