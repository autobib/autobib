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
    let ab = TestState::init()?;

    let mut cmd = ab.cmd()?;
    cmd.arg("help").assert().success();

    ab.close()
}

/// Check that `autobib get` returns what is expected.
#[test]
fn get() -> Result<()> {
    let ab = TestState::init()?;

    let mut cmd = ab.cmd()?;
    let predicate_file = predicate::path::eq_file(Path::new("tests/resources/get/stdout.txt"))
        .utf8()
        .unwrap();
    cmd.args(["get", "zbl:1337.28015", "arxiv:1212.1873", "mr:3224722"]);
    cmd.assert()
        .success()
        .stdout(predicate_file)
        .stderr(predicate::str::is_empty());

    ab.close()
}

/// Check that `autobib source` returns what is expected.
#[test]
fn source() -> Result<()> {
    let ab = TestState::init()?;

    let mut cmd = ab.cmd()?;
    cmd.arg("source").arg("tests/resources/source/main.tex");
    let predicate_file = predicate::path::eq_file(Path::new("tests/resources/source/stdout.txt"))
        .utf8()
        .unwrap();
    cmd.assert()
        .success()
        .stdout(predicate_file)
        .stderr(predicate::str::is_empty());

    ab.close()
}

/// Check that `autobib get` fails correctly when the resource does not exist.
#[test]
fn get_null() -> Result<()> {
    let ab = TestState::init()?;

    let mut cmd = ab.cmd()?;
    cmd.args(["get", "zbl:9999.28015"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Null record"));

    ab.close()
}

/// Check that `autobib local`.
#[test]
fn local() -> Result<()> {
    let ab = TestState::init()?;

    let mut cmd = ab.cmd()?;
    cmd.args([
        "local",
        "first",
        "--from",
        "tests/resources/local/first.bib",
    ]);
    cmd.assert().success();

    let mut cmd = ab.cmd()?;
    cmd.args(["get", "local:first"]);
    let predicate_file = predicate::path::eq_file(Path::new("tests/resources/local/stdout.txt"))
        .utf8()
        .unwrap();
    cmd.assert().success().stdout(predicate_file);

    ab.close()
}

/// Check that `autobib alias` works as expected.
#[test]
fn alias() -> Result<()> {
    let ab = TestState::init()?;

    let mut cmd = ab.cmd()?;
    cmd.args([
        "local",
        "first",
        "--from",
        "tests/resources/local/first.bib",
    ]);
    cmd.assert().success();

    let mut cmd = ab.cmd()?;
    cmd.args(["get", "local:first"]);
    cmd.assert().success();

    let mut cmd = ab.cmd()?;
    cmd.args(["alias", "add", "my_alias", "local:first"]);
    cmd.assert().success();

    let mut cmd = ab.cmd()?;
    cmd.arg("get").arg("my_alias");
    let predicate_file = predicate::path::eq_file(Path::new("tests/resources/alias/stdout.txt"))
        .utf8()
        .unwrap();
    cmd.assert().success().stdout(predicate_file);

    let mut cmd = ab.cmd()?;
    cmd.args(["alias", "rename", "my_alias", "new_alias"]);
    cmd.assert().success();

    let mut cmd = ab.cmd()?;
    cmd.args(["get", "new_alias"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("@book{new_alias"));

    let mut cmd = ab.cmd()?;
    cmd.args(["alias", "delete", "new_alias"]);
    cmd.assert().success();

    let mut cmd = ab.cmd()?;
    cmd.args(["get", "my_alias"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Null alias"));

    let mut cmd = ab.cmd()?;
    cmd.args(["get", "new_alias"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Null alias"));

    let mut cmd = ab.cmd()?;
    cmd.args(["alias", "delete", "my_alias"]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "Could not delete alias which does not exist",
    ));

    ab.close()
}

/// Check that `autobib alias` works as expected with null and existing remote records.
#[test]
fn alias_remote() -> Result<()> {
    let ab = TestState::init()?;

    let mut cmd = ab.cmd()?;
    cmd.args(["alias", "add", "al", "zbmath:06346461"]);
    cmd.assert().success();

    let mut cmd = ab.cmd()?;
    cmd.args(["get", "al"]);
    let predicate_file =
        predicate::path::eq_file(Path::new("tests/resources/alias_remote/stdout.txt"))
            .utf8()
            .unwrap();
    cmd.assert().success().stdout(predicate_file);

    let mut cmd = ab.cmd()?;
    cmd.args(["alias", "add", "a2", "zbmath:96346461"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Null record"));

    let mut cmd = ab.cmd()?;
    cmd.args(["get", "a2"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Null alias"));

    ab.close()
}

/// Check that `autobib get` warns if there are multiple references to the same key
#[test]
fn repeat() -> Result<()> {
    let ab = TestState::init()?;

    let mut cmd = ab.cmd()?;
    cmd.args(["get", "zbmath:06346461", "zbl:1337.28015"]);
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("Multiple keys for "));

    let mut cmd = ab.cmd()?;
    cmd.args(["alias", "add", "a", "zbl:1337.28015"]);
    cmd.assert().success();

    let mut cmd = ab.cmd()?;
    cmd.args(["get", "zbmath:06346461", "a"]);
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("Multiple keys for "));

    let mut cmd = ab.cmd()?;
    cmd.args(["get", "a", "a"]);
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("Multiple keys for "));

    ab.close()
}
