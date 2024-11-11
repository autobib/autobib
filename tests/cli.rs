use assert_cmd::prelude::*;
use assert_fs::fixture::NamedTempFile;
use predicates::prelude::*;

use std::{path::Path, process::Command};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct TestState {
    database: NamedTempFile,
}

impl TestState {
    fn init() -> Result<Self> {
        Ok(Self {
            database: NamedTempFile::new("records.db")?,
        })
    }

    fn cmd(&self) -> Result<Command> {
        let mut cmd = Command::cargo_bin("autobib").unwrap();
        cmd.arg("--database").arg(self.database.as_ref());
        Ok(cmd)
    }

    fn close(self) -> Result<()> {
        Ok(())
    }
}

/// Check that the binary is working properly so we can run `autobib help`.
#[test]
fn runs_help() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.arg("help").assert().success();

    s.close()
}

/// Check that `autobib get` returns what is expected.
#[test]
fn get() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    let predicate_file = predicate::path::eq_file(Path::new("tests/resources/get/stdout.txt"))
        .utf8()
        .unwrap();
    cmd.args(["get", "zbl:1337.28015", "arxiv:1212.1873", "mr:3224722"]);
    cmd.assert()
        .success()
        .stdout(predicate_file)
        .stderr(predicate::str::is_empty());

    s.close()
}

/// Check that `autobib source` returns what is expected.
#[test]
fn source() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.arg("source").arg("tests/resources/source/main.tex");
    let predicate_file = predicate::path::eq_file(Path::new("tests/resources/source/stdout.txt"))
        .utf8()
        .unwrap();
    cmd.assert()
        .success()
        .stdout(predicate_file)
        .stderr(predicate::str::is_empty());

    s.close()
}

/// Check that `autobib get` fails correctly when the resource does not exist.
#[test]
fn get_null() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args(["get", "zbl:9999.28015"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Null record"));

    let mut cmd = s.cmd()?;
    cmd.args(["get", "--ignore-null", "zbl:9999.28015"]);
    cmd.assert().success().stderr(predicate::str::is_empty());

    s.close()
}

/// Check that `autobib local` works as expected.
#[test]
fn local() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args([
        "local",
        "first",
        "--from",
        "tests/resources/local/first.bib",
        "--no-edit",
    ]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "local:first"]);
    let predicate_file = predicate::path::eq_file(Path::new("tests/resources/local/stdout.txt"))
        .utf8()
        .unwrap();
    cmd.assert().success().stdout(predicate_file);

    let mut cmd = s.cmd()?;
    cmd.args(["local", "second", "--no-edit"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "local:second"]);
    let predicate_file =
        predicate::path::eq_file(Path::new("tests/resources/local/stdout_short.txt"))
            .utf8()
            .unwrap();
    cmd.assert().success().stdout(predicate_file);

    let mut cmd = s.cmd()?;
    cmd.args(["local", " \n"]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "local sub-id must contain non-whitespace characters",
    ));

    let mut cmd = s.cmd()?;
    cmd.args(["local", ":"]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "local sub-id must not contain a colon",
    ));

    s.close()
}

/// Check that `autobib alias` works as expected.
#[test]
fn alias() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args([
        "local",
        "first",
        "--from",
        "tests/resources/local/first.bib",
        "--no-edit",
    ]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "local:first"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "my_alias", "local:first"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["local", "second", "--no-edit"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "my_alias", "local:second"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Alias already exists"));

    let mut cmd = s.cmd()?;
    cmd.arg("get").arg("my_alias");
    let predicate_file = predicate::path::eq_file(Path::new("tests/resources/alias/stdout.txt"))
        .utf8()
        .unwrap();
    cmd.assert().success().stdout(predicate_file);

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "rename", "my_alias", "new_alias"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "new_alias"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("@book{new_alias"));

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "delete", "new_alias"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "my_alias"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Undefined alias"));

    let mut cmd = s.cmd()?;
    cmd.args(["get", "new_alias"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Undefined alias"));

    let mut cmd = s.cmd()?;
    cmd.args(["get", "--ignore-null", "new_alias"]);
    cmd.assert().success().stderr(predicate::str::is_empty());

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "delete", "my_alias"]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "Could not delete alias which does not exist",
    ));

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "  ", "not_an_alias"]);
    cmd.assert().failure().stderr(
        predicate::str::contains("invalid value '  ' for '<ALIAS>'").and(predicate::str::contains(
            "alias must contain non-whitespace characters",
        )),
    );

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "\n\t", "not_an_alias"]);
    cmd.assert().failure().stderr(
        predicate::str::contains("invalid value '\n\t' for '<ALIAS>'").and(
            predicate::str::contains("alias must contain non-whitespace characters"),
        ),
    );

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "has ws", "not_an_alias"]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "Cannot create alias for undefined alias",
    ));

    s.close()
}

/// Check that `autobib alias` works as expected with null and existing remote records.
#[test]
fn alias_remote() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "al", "zbmath:06346461"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "al"]);
    let predicate_file =
        predicate::path::eq_file(Path::new("tests/resources/alias_remote/stdout.txt"))
            .utf8()
            .unwrap();
    cmd.assert().success().stdout(predicate_file);

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "a2", "zbmath:96346461"]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "Cannot create alias for null record",
    ));

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "a2", "alias-does-not-exist"]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "Cannot create alias for undefined alias",
    ));

    let mut cmd = s.cmd()?;
    cmd.args(["get", "a2"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Undefined alias"));

    s.close()
}

/// Check that `autobib get` validates BibTeX citation keys and suggests alternatives on failure.
#[test]
fn bibtex_key_validation() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args([
        "alias",
        "add",
        "cst1989",
        "doi:10.1016/0021-8693(89)90256-1",
    ]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "doi:10.1016/0021-8693(89)90256-1"]);
    cmd.assert().failure().stderr(
        predicate::str::contains("identifier contains invalid character")
            .and(predicate::str::contains("cst1989")),
    );

    let mut cmd = s.cmd()?;
    cmd.args(["get", "cst1989"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args([
        "get",
        "--retrieve-only",
        "doi:10.1016/0021-8693(89)90256-1",
        "cst1989",
    ]);
    cmd.assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "has ws", "cst1989"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "has ws"]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "identifier contains invalid character",
    ));

    s.close()
}

/// Test deletion, including of aliases.
#[test]
fn delete() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args([
        "local",
        "first",
        "--from",
        "tests/resources/local/first.bib",
        "--no-edit",
    ]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "local:first"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "my_alias", "local:first"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["delete", "--force", "local:first"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "my_alias"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Undefined alias"));

    let mut cmd = s.cmd()?;
    cmd.args(["get", "local:first"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Undefined local record"));

    s.close()
}

/// Test citation key listing.
#[test]
fn list() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args([
        "local",
        "first",
        "--from",
        "tests/resources/local/first.bib",
        "--no-edit",
    ]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "my_alias", "local:first"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "zbl:1337.28015"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["util", "list"]);
    cmd.assert().success().stdout(
        predicate::str::contains("zbmath:06346461").and(predicate::str::contains("my_alias")),
    );

    let mut cmd = s.cmd()?;
    cmd.args(["util", "list", "--canonical"]);
    cmd.assert().success().stdout(
        predicate::str::contains("my_alias")
            .not()
            .and(predicate::str::contains("local:first")),
    );

    s.close()
}

#[test]
fn info() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args(["info", "zbl:1337.28015", "-r", "canonical"]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "Cannot obtain report for record not in database",
    ));

    let mut cmd = s.cmd()?;
    cmd.args(["get", "zbl:1337.28015"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["info", "zbl:1337.28015", "--report", "canonical"]);
    cmd.assert().success().stdout("zbmath:06346461\n");

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "%", "zbmath:06346461"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["info", "zbl:1337.28015", "-r", "equivalent"]);
    cmd.assert().success().stdout(
        predicate::str::contains("%")
            .and(predicate::str::contains("zbmath:06346461"))
            .and(predicate::str::contains("zbl:1337.28015")),
    );

    let mut cmd = s.cmd()?;
    cmd.args(["info", "%", "-r", "valid"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Invalid BibTeX"));

    let mut cmd = s.cmd()?;
    cmd.args(["info", "%"]);
    cmd.assert().success().stdout(
        predicate::str::contains("Data last modified:")
            .and(predicate::str::contains("Equivalent references:"))
            .and(predicate::str::contains("Canonical: zbmath:06346461\n"))
            .and(predicate::str::contains("Valid BibTeX? no")),
    );

    s.close()
}

#[test]
fn edit() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args(["edit", "zbl:9999.28015"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Cannot edit null record"));

    let mut cmd = s.cmd()?;
    cmd.args(["edit", "my_alias"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Cannot edit undefined alias"));

    s.close()
}

#[test]
fn update() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args(["update", "zbmath:06346461"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("does not exist in database"));

    s.close()
}

#[test]
fn consistency() -> Result<()> {
    use rusqlite::Connection;

    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args([
        "get",
        "--retrieve-only",
        "zbmath:06346461",
        "zbl:1337.28015",
        "mr:3224722",
    ]);
    cmd.assert().success();

    // perform some destructive changes to the database
    let conn = Connection::open(s.database.path())?;
    conn.pragma_update(None, "foreign_keys", 0)?;
    conn.prepare("DELETE FROM Records WHERE record_id = 'zbmath:06346461'")?
        .execute(())?;
    conn.prepare("DELETE FROM CitationKeys WHERE name = 'mr:3224722'")?
        .execute(())?;
    drop(conn);

    // check that things are broken
    let mut cmd = s.cmd()?;
    cmd.args(["get", "mr:3224722"]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "UNIQUE constraint failed: Records.record_id",
    ));
    let mut cmd = s.cmd()?;
    cmd.args(["get", "zbmath:06346461"]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "Database error: SQLite error: Query returned no rows",
    ));

    // check that the error report is correct
    let mut cmd = s.cmd()?;
    cmd.args(["util", "check"]);
    cmd.assert().failure().stderr(
        predicate::str::contains(
            "There are 2 citation keys which reference records which do not exist in the database.",
        )
        .and(predicate::str::contains(
            "Record row '2' with record id 'mr:3224722' does not have corresponding key",
        )),
    );

    // fix things
    let mut cmd = s.cmd()?;
    cmd.args(["util", "check", "--fix"]);
    cmd.assert().success().stderr(
        predicate::str::contains(
            "Repairing dangling record by inserting or overwriting existing citation key",
        )
        .and(predicate::str::contains(
            "Deleting citation keys which do not reference records:",
        ))
        .and(predicate::str::contains("zbl:1337.28015"))
        .and(predicate::str::contains("zbmath:06346461")),
    );

    // check that things are fixed
    let mut cmd = s.cmd()?;
    cmd.args(["get", "mr:3224722", "zbmath:06346461"]);
    cmd.assert().success();

    s.close()
}

/// Check that `autobib get` warns if there are multiple references to the same key
#[test]
fn repeat() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args(["get", "zbmath:06346461", "zbl:1337.28015"]);
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("Multiple keys for "));

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "a", "zbl:1337.28015"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "zbmath:06346461", "a"]);
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("Multiple keys for "));

    let mut cmd = s.cmd()?;
    cmd.args(["get", "a", "a"]);
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("Multiple keys for "));

    s.close()
}

/// Test identifiers which have previously caused errors
#[test]
fn test_identifier_exceptions() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args(["get", "arxiv:2112.04570"]);
    cmd.assert().success();

    s.close()
}
