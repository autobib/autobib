use assert_cmd::prelude::*;
use assert_fs::fixture::{ChildPath, NamedTempFile, PathChild, TempDir};
use assert_fs::prelude::*;
use predicates::prelude::*;

use std::{fs, path::Path, process::Command};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct TestState {
    database: NamedTempFile,
    config: NamedTempFile,
    attach_dir: TempDir,
}

impl TestState {
    fn init() -> Result<Self> {
        let config = NamedTempFile::new("config.toml")?;
        fs::write(config.as_ref(), "")?;
        Ok(Self {
            database: NamedTempFile::new("records.db")?,
            config,
            attach_dir: TempDir::new()?,
        })
    }

    fn cmd(&self) -> Result<Command> {
        let mut cmd = Command::cargo_bin("autobib").unwrap();
        cmd.arg("--database")
            .arg(self.database.as_ref())
            .arg("--config")
            .arg(self.config.as_ref())
            .arg("--attachments-dir")
            .arg(self.attach_dir.as_ref())
            .arg("--no-interactive");
        Ok(cmd)
    }

    fn attachment<P: AsRef<Path>>(&self, path: P) -> ChildPath {
        self.attach_dir.child(path)
    }

    fn set_config<P: AsRef<Path>>(&self, config: P) -> Result<()> {
        fs::copy(config, self.config.as_ref())?;
        Ok(())
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

/// Check that we correctly suggest alternative keys
#[test]
fn suggest_alternatives() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args(["get", "zbl:math/0001001"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("arxiv:math/0001001"));
    Ok(())
}

/// Check that `autobib get` returns what is expected.
#[test]
fn get() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    let predicate_file = predicate::path::eq_file(Path::new("tests/resources/get/stdout.txt"))
        .utf8()
        .unwrap();
    cmd.args([
        "get",
        "zbl:1337.28015",
        "zbl:1285.28011",
        "arxiv:1212.1873",
        "mr:3224722",
    ]);
    cmd.assert()
        .success()
        .stdout(predicate_file)
        .stderr(predicate::str::is_empty());

    let mut cmd = s.cmd()?;
    cmd.args(["--read-only", "get", "arxiv:1212.1873"]);
    cmd.assert().success();

    s.close()
}

/// Check that `autobib get --append` returns what is expected.
#[test]
fn get_append() -> Result<()> {
    let s = TestState::init()?;

    let output = NamedTempFile::new("out.bib")?;
    output.write_str("@preprint{arxiv:1212.1873,}\n")?;

    let mut cmd = s.cmd()?;

    cmd.args([
        "get",
        "zbl:1337.28015",
        "arxiv:1212.1873",
        "--out",
        &output.to_string_lossy(),
        "--append",
    ]);

    cmd.assert().success().stderr(predicate::str::is_empty());

    let predicate_file =
        predicate::path::eq_file(Path::new("tests/resources/get_append/stdout.txt"))
            .utf8()
            .unwrap();

    assert!(predicate_file.eval(output.as_ref()));

    s.close()
}

/// Check that `autobib source` returns what is expected.
#[test]
fn source() -> Result<()> {
    let s = TestState::init()?;

    let predicate_file = predicate::path::eq_file(Path::new("tests/resources/source/stdout.txt"))
        .utf8()
        .unwrap();

    let mut cmd = s.cmd()?;
    cmd.args(["source", "tests/resources/source/main.tex"]);
    cmd.assert()
        .success()
        .stdout(predicate_file.clone())
        .stderr(predicate::str::is_empty());

    let mut cmd = s.cmd()?;
    cmd.args(["--read-only", "source", "tests/resources/source/main.tex"]);
    cmd.assert()
        .success()
        .stdout(predicate_file.clone())
        .stderr(predicate::str::is_empty());

    let mut cmd = s.cmd()?;
    cmd.args(["source", "--stdin", "tex"])
        .stdin(fs::File::open("tests/resources/source/main.tex")?);
    cmd.assert()
        .success()
        .stdout(predicate_file)
        .stderr(predicate::str::is_empty());

    s.close()
}

/// Check that `autobib source --print-keys` works.
#[test]
fn source_keys_only() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args(["source", "tests/resources/source/main.tex", "--print-keys"]);

    s.close()
}

/// Check that the `--skip*` and `--append` options for `autobib source`
/// work as expected
#[test]
fn source_skip() -> Result<()> {
    let s = TestState::init()?;

    let output = NamedTempFile::new("out.bib")?;
    output.write_str("@preprint{arxiv:1212.1873,}\n")?;

    let mut cmd = s.cmd()?;

    cmd.arg("source")
        .arg("tests/resources/source_skip/main.tex");
    cmd.args([
        "--skip",
        "isbn:9781119942399",
        "--skip-from",
        "tests/resources/source_skip/skip.tex",
        "--skip-from",
        "tests/resources/source_skip/skip.bib",
        "--out",
        &output.to_string_lossy(),
        "--append",
    ]);

    cmd.assert().success().stderr(predicate::str::is_empty());

    let predicate_file =
        predicate::path::eq_file(Path::new("tests/resources/source_skip/stdout.txt"))
            .utf8()
            .unwrap();

    assert!(predicate_file.eval(output.as_ref()));

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
    ]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["--read-only", "local", "second"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used in read-only mode"));

    let mut cmd = s.cmd()?;
    cmd.args(["get", "local:first"]);
    let predicate_file = predicate::path::eq_file(Path::new("tests/resources/local/stdout.txt"))
        .utf8()
        .unwrap();
    cmd.assert().success().stdout(predicate_file);

    let mut cmd = s.cmd()?;
    cmd.args(["local", "first"]);
    cmd.assert().failure();

    let mut cmd = s.cmd()?;
    cmd.args([
        "local",
        "first",
        "--from",
        "tests/resources/local/first.bib",
    ]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "Local record 'local:first' already exists",
    ));

    let mut cmd = s.cmd()?;
    cmd.args(["local", "second"]);
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
    ]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "local:first"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "my_alias", "local:first"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["local", "second"]);
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
        predicate::str::contains("Identifier contains invalid character")
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
        "Identifier contains invalid character",
    ));

    s.close()
}

/// Test deletion, including of aliases.
#[test]
fn delete() -> Result<()> {
    let s = TestState::init()?;

    // single deletion OK even without `--force`
    let mut cmd = s.cmd()?;
    cmd.args(["get", "mr:3224722"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["delete", "mr:3224722"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["delete", "mr:3224722"]);
    cmd.assert().failure();

    // multi deletion fails without `--force`
    let mut cmd = s.cmd()?;
    cmd.args([
        "local",
        "first",
        "--from",
        "tests/resources/local/first.bib",
    ]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "first", "local:first"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "local:first"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "my_alias", "local:first"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["delete", "local:first"]);
    cmd.assert().failure().stderr(
        predicate::str::contains("has associated keys which are not requested for deletion")
            .and(predicate::str::contains("my_alias"))
            .and(predicate::str::contains("first")),
    );

    let mut cmd = s.cmd()?;
    cmd.args(["delete", "my_alias", "first"]);
    cmd.assert().failure().stderr(
        predicate::str::contains("has associated keys which are not requested for deletion")
            .and(predicate::str::contains("local:first")),
    );

    let mut cmd = s.cmd()?;
    cmd.args(["get", "local:first"]);
    cmd.assert().success();

    // multi deletion succeeds with `--force`
    let mut cmd = s.cmd()?;
    cmd.args(["delete", "--force", "local:first", "first"]);
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
        .stderr(predicate::str::contains("Unexpected local record"));

    // multi deletion succeeds if all keys are passed
    let mut cmd = s.cmd()?;
    cmd.args([
        "local",
        "first",
        "--from",
        "tests/resources/local/first.bib",
    ]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "first", "local:first"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "my_alias", "local:first"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["delete", "local:first", "my_alias", "first"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "my_alias"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Undefined alias"));

    let mut cmd = s.cmd()?;
    cmd.args(["get", "first"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Undefined alias"));

    let mut cmd = s.cmd()?;
    cmd.args(["get", "local:first"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Unexpected local record"));

    // deletions are deduplicated automatically
    let mut cmd = s.cmd()?;
    cmd.args([
        "local",
        "first",
        "--from",
        "tests/resources/local/first.bib",
    ]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["alias", "add", "first", "local:first"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args([
        "delete",
        "local:first",
        "first",
        "local:first",
        "local:first",
    ]);
    cmd.assert().success();

    // do not emit error for forced deletion of a record which does not exist
    let mut cmd = s.cmd()?;
    cmd.args(["delete", "arxiv:1212.1873"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Identifier not in database"));

    let mut cmd = s.cmd()?;
    cmd.args(["delete", "--force", "arxiv:1212.1873"]);
    cmd.assert().success();

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
    cmd.args(["--read-only", "util", "list"]);
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
    cmd.args([
        "--read-only",
        "info",
        "zbl:1337.28015",
        "--report",
        "canonical",
    ]);
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
fn test_attach() -> Result<()> {
    let s = TestState::init()?;

    let temp = assert_fs::NamedTempFile::new("attachment.txt")?;
    let temp_contents = "test\ncontents";
    temp.write_str(temp_contents)?;

    let attachment_file = s.attachment("zbmath/JX/TT/CT/GA3DGNBWGQ3DC===/attachment.txt");

    let mut cmd = s.cmd()?;
    cmd.args(["attach", "zbl:1337.28015"]);
    cmd.arg(temp.as_ref());
    cmd.assert().success();

    attachment_file.assert(predicate::eq(temp_contents));

    let mut cmd = s.cmd()?;
    cmd.args(["attach", "zbl:1337.28015"]);
    cmd.arg(temp.as_ref());
    cmd.args(["--rename", "attach2.txt"]);
    cmd.assert().success();

    s.attachment("zbmath/JX/TT/CT/GA3DGNBWGQ3DC===/attach2.txt")
        .assert(predicate::eq(temp_contents));

    let mut cmd = s.cmd()?;
    cmd.args(["attach", "zbl:1337.28015"]);
    cmd.arg(temp.as_ref());
    cmd.args(["--rename", ".."]);
    cmd.assert().failure();

    let mut cmd = s.cmd()?;
    cmd.args(["attach", "zbl:1337.28015"]);
    cmd.arg(temp.as_ref());
    cmd.args(["--rename", "/invalid"]);
    cmd.assert().failure();

    let mut cmd = s.cmd()?;
    cmd.args(["attach", "zbl:1337.28015"]);
    cmd.arg(temp.as_ref());
    cmd.args(["--rename", ""]);
    cmd.assert().failure();

    let mut cmd = s.cmd()?;
    cmd.args(["attach", "zbl:1337.28015"]);
    cmd.arg(temp.as_ref());
    cmd.args(["--rename", "."]);
    cmd.assert().failure();

    temp.close()?;
    s.close()
}

/// Check that `autobib path` always returns the same values.
#[test]
fn test_path_platform_consistency() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args(["path", "zbl:1337.28015"]);

    #[cfg(windows)]
    let value = "\\zbmath\\JX\\TT\\CT\\GA3DGNBWGQ3DC===\\\n";

    #[cfg(not(windows))]
    let value = "/zbmath/JX/TT/CT/GA3DGNBWGQ3DC===/\n";

    cmd.assert()
        .success()
        .stdout(predicate::str::ends_with(value));

    let mut cmd = s.cmd()?;
    cmd.args([
        "alias",
        "add",
        "my-alias",
        "doi:10.1016/0021-8693(89)90256-1",
    ]);
    cmd.assert().success();

    #[cfg(windows)]
    let value = "\\doi\\XN\\UL\\PE\\GEYC4MJQGE3C6MBQGIYS2OBWHEZSQOBZFE4TAMRVGYWTC===\\\n";

    #[cfg(not(windows))]
    let value = "/doi/XN/UL/PE/GEYC4MJQGE3C6MBQGIYS2OBWHEZSQOBZFE4TAMRVGYWTC===/\n";

    let mut cmd = s.cmd()?;
    cmd.args(["path", "my-alias"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::ends_with(value));

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

    let predicate_file =
        predicate::path::eq_file(Path::new("tests/resources/edit/stdout_unedited.txt"))
            .utf8()
            .unwrap();
    let mut cmd = s.cmd()?;
    cmd.args(["get", "mr:3224722"]);
    cmd.assert().success().stdout(predicate_file);

    let mut cmd = s.cmd()?;
    cmd.args(["edit", "--set-eprint=zbl,doi", "mr:3224722"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["edit", "mr:3224722", "--normalize-whitespace"]);
    cmd.assert().success();

    let predicate_file = predicate::path::eq_file(Path::new("tests/resources/edit/stdout.txt"))
        .utf8()
        .unwrap();
    let mut cmd = s.cmd()?;
    cmd.args(["get", "mr:3224722"]);
    cmd.assert().success().stdout(predicate_file);

    s.close()
}

#[test]
fn update() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args(["update", "zbmath:06346461"]);
    cmd.assert().failure().stderr(
        predicate::str::contains("does not exist in database")
            .and(predicate::str::contains("Use `autobib get`")),
    );

    s.close()
}

#[test]
fn update_local() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args(["local", "one"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "local:one"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["update", "local:one"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Unexpected local record"));

    let mut cmd = s.cmd()?;
    cmd.args(["update", "local:two"]);
    cmd.assert().failure().stderr(
        predicate::str::contains("does not exist in database")
            .and(predicate::str::contains("Use `autobib get`").not()),
    );

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

#[test]
fn config() -> Result<()> {
    let s = TestState::init()?;

    s.set_config(Path::new("tests/resources/config/malformed.toml"))?;
    let mut cmd = s.cmd()?;
    cmd.arg("get");
    cmd.assert().failure();

    s.set_config(Path::new("tests/resources/config/extra.toml"))?;
    let mut cmd = s.cmd()?;
    cmd.arg("get");
    cmd.assert().failure();

    s.close()
}

/// Check that the `on_insert` methods work as expected.
#[test]
fn on_insert() -> Result<()> {
    let s = TestState::init()?;

    s.set_config(Path::new("tests/resources/on_insert/config.toml"))?;

    let predicate_file =
        predicate::path::eq_file(Path::new("tests/resources/on_insert/stdout.txt"))
            .utf8()
            .unwrap();
    let mut cmd = s.cmd()?;
    cmd.args(["get", "mr:3224722"]);
    cmd.assert().success().stdout(predicate_file);

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

// #[test]
// fn test_merge() -> Result<()> {
//     let s = TestState::init()?;

//     let mut cmd = s.cmd()?;
//     cmd.args(["get", "zbl:1337.28015", "arxiv:1212.1873", "mr:3224722"]);
//     cmd.assert().success();

//     let mut cmd = s.cmd()?;
//     cmd.args(["alias", "add", "a", "arxiv:1212.1873"]);
//     cmd.assert().success();

//     let mut cmd = s.cmd()?;
//     cmd.args(["merge", "mr:3224722", "a", "zbl:1337.28015"]);
//     cmd.assert().success();

//     let predicate_file = predicate::path::eq_file(Path::new("tests/resources/merge/stdout.txt"))
//         .utf8()
//         .unwrap();

//     let mut cmd = s.cmd()?;
//     cmd.args(["get", "zbmath:06346461"]);
//     cmd.assert().success().stdout(predicate_file);

//     s.close()
// }

#[test]
fn test_quiet_returns_error() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args(["-q", "-q", "get", "::invalid"]);
    cmd.assert().failure();

    s.close()
}

#[test]
fn test_cache_evict() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args(["get", "zbmath:96346461"]);
    cmd.assert().failure();

    let mut cmd = s.cmd()?;
    cmd.args(["-v", "util", "evict", "--max-age", "10000"]);
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("Removed 0 cached null"));

    let mut cmd = s.cmd()?;
    cmd.args(["-v", "util", "evict"]);
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("Removed 1 cached null"));

    s.close()
}

#[test]
fn test_normalize() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args(["get", "zbmath:6346461"]);
    cmd.assert().success().stdout(
        predicate::path::eq_file(Path::new("tests/resources/normalize/stdout.txt"))
            .utf8()
            .unwrap(),
    );

    let mut cmd = s.cmd()?;
    cmd.args(["info", "zbmath:1111111"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("converted from 'zbmath:1111111'"));

    s.close()
}

#[test]
fn test_strip_journal_series() -> Result<()> {
    let s = TestState::init()?;

    s.set_config(Path::new(
        "tests/resources/strip_journal_series/config.toml",
    ))?;

    let mut cmd = s.cmd()?;
    cmd.args(["get", "zbl:1337.28015"]);
    cmd.assert().success().stdout(
        predicate::path::eq_file(Path::new("tests/resources/strip_journal_series/stdout.txt"))
            .utf8()
            .unwrap(),
    );

    s.close()
}

#[test]
fn test_auto_alias() -> Result<()> {
    let s = TestState::init()?;

    s.set_config(Path::new("tests/resources/auto_alias/config.toml"))?;

    let mut cmd = s.cmd()?;
    cmd.args(["get", "zbMATH06346461"]);
    cmd.assert().success().stdout(
        predicate::path::eq_file(Path::new("tests/resources/auto_alias/stdout.txt"))
            .utf8()
            .unwrap(),
    );

    let mut cmd = s.cmd()?;
    cmd.args(["get", "zbMATH6346461"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Undefined alias"));

    let mut cmd = s.cmd()?;
    cmd.args(["get", "zbl:1337.28015"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["info", "zbl:1337.28015", "--report", "equivalent"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("zbMATH06346461"));

    let mut cmd = s.cmd()?;
    cmd.args(["get", "mr:3224722"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "MR3224722"]);
    cmd.assert().success().stdout(
        predicate::path::eq_file(Path::new("tests/resources/auto_alias/stdout_mr.txt"))
            .utf8()
            .unwrap(),
    );

    let mut cmd = s.cmd()?;
    cmd.args(["info", "MR3224722", "--report", "equivalent"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("mr:3224722"));

    s.close()
}

#[test]
fn import_local() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args(["import", "tests/resources/import/file.bib"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "attainable-assouad-spectra"]);
    cmd.assert().success().stdout(
        predicate::path::eq_file(Path::new("tests/resources/import/stdout_local.txt"))
            .utf8()
            .unwrap(),
    );

    let mut cmd = s.cmd()?;
    cmd.args(["get", "zbmath:7937992"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("zbmath = {07937992}"));

    s.close()
}

#[test]
fn import_determine_key_no_match() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args([
        "import",
        "tests/resources/import/file.bib",
        "-m",
        "determine-key",
    ]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "attainable-assouad-spectra"]);
    cmd.assert().success().stdout(
        predicate::path::eq_file(Path::new("tests/resources/import/stdout_local.txt"))
            .utf8()
            .unwrap(),
    );

    // the remote record is different, since `zbmath` was not set as a preferred provider and there
    // was no matching alias transform
    let mut cmd = s.cmd()?;
    cmd.args(["get", "zbmath:7937992", "local:zbMATH06346461"]);
    cmd.assert().success().stdout(
        predicate::path::eq_file(Path::new("tests/resources/import/stdout_remote.txt"))
            .utf8()
            .unwrap(),
    );

    s.close()
}

#[test]
fn import_determine_key_match() -> Result<()> {
    let s = TestState::init()?;

    // set configuration to allow keys to be determined successfully
    s.set_config("tests/resources/import/config.toml")?;

    let mut cmd = s.cmd()?;
    cmd.args([
        "import",
        "tests/resources/import/file.bib",
        "-m",
        "determine-key",
    ]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "attainable-assouad-spectra"]);
    cmd.assert().success().stdout(
        predicate::path::eq_file(Path::new("tests/resources/import/stdout_local.txt"))
            .utf8()
            .unwrap(),
    );

    // this time, the records were successfully found locally so no remote retrieval is required
    let mut cmd = s.cmd()?;
    cmd.args(["get", "local:zbMATH06346461"]);
    cmd.assert().failure();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "zbmath:07937992", "zbmath:06346461"]);
    cmd.assert().success().stdout(
        predicate::path::eq_file(Path::new("tests/resources/import/stdout_local_2.txt"))
            .utf8()
            .unwrap(),
    );

    s.close()
}

#[test]
fn import_retrieve() -> Result<()> {
    let s = TestState::init()?;

    // set configuration to allow keys to be determined successfully
    s.set_config("tests/resources/import/config.toml")?;

    let mut cmd = s.cmd()?;
    cmd.args([
        "import",
        "tests/resources/import/file.bib",
        "-mr",
        "-n",
        "prefer-incoming",
    ]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["get", "attainable-assouad-spectra", "zbl:1337.28015"]);
    cmd.assert().success().stdout(
        predicate::path::eq_file(Path::new("tests/resources/import/stdout_retrieve.txt"))
            .utf8()
            .unwrap(),
    );

    s.close()
}

#[test]
fn read_only() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args(["get", "zbl:1337.28015"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["--read-only", "get", "zbl:1337.28015"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["--read-only", "get", "arxiv:1212.1873"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Database does not contain key"));

    for arg in ["check", "list"] {
        let mut cmd = s.cmd()?;
        cmd.args(["--read-only", "util", arg]);
        cmd.assert().success();
    }

    let mut cmd = s.cmd()?;
    cmd.args(["--read-only", "info", "zbl:1337.28015"]);
    cmd.assert().success();

    let mut cmd = s.cmd()?;
    cmd.args(["--read-only", "info", "arxiv:1212.1873"]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "Cannot obtain report for record not in database",
    ));

    Ok(())
}

#[test]
fn import_retrieve_only() -> Result<()> {
    let s = TestState::init()?;

    let mut cmd = s.cmd()?;
    cmd.args([
        "import",
        "tests/resources/import/file.bib",
        "-m",
        "retrieve-only",
        "--log-failures",
    ]);
    cmd.assert().failure().stdout(
        predicate::str::contains("zbMATH06346461")
            .and(predicate::str::contains("attainable-assouad-spectra")),
    );

    s.close()
}

macro_rules! test_provider_success {
    ($name:ident, $target:expr) => {
        /// Check that `autobib get` succeeds
        #[test]
        fn $name() -> Result<()> {
            let s = TestState::init()?;

            let mut cmd = s.cmd()?;
            cmd.args(["-vv", "get", $target]);
            cmd.assert().success();

            s.close()
        }
    };
}

test_provider_success!(arxiv_provider, "arxiv:1212.1873");
test_provider_success!(doi_provider, "doi:10.4007/annals.2014.180.2.7");
test_provider_success!(isbn_provider, "isbn:9781119942399");
test_provider_success!(jfm_provider, "jfm:60.0017.02");
test_provider_success!(mr_provider, "mr:3224722");
test_provider_success!(ol_provider, "ol:31159704M");
test_provider_success!(zbl_provider, "zbl:1337.28015");
test_provider_success!(zbmath_provider, "zbmath:7937992");
