CREATE TABLE "Records" (
  "rev" INTEGER PRIMARY KEY,
  "canonical" TEXT NOT NULL,
  "data" BLOB NOT NULL,
  "modified" TEXT NOT NULL,
  "variant" INTEGER NOT NULL DEFAULT 0,
  "parent_rev" INTEGER REFERENCES "Records"(rev)
    ON UPDATE RESTRICT
    ON DELETE SET NULL
) STRICT
