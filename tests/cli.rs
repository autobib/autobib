use assert_cmd::cmd::Command;
use assert_fs::{
    assert::PathAssert,
    fixture::{ChildPath, FileWriteStr, NamedTempFile, PathChild, TempDir},
};
use predicates::{Predicate, boolean::PredicateBooleanExt, prelude::predicate, str::contains};

use std::{
    fs,
    path::{Path, PathBuf},
};

static AUTOBIB_LOCKFILE: &str = ".autobib_lock";

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn native_path<const N: usize>(parts: [&str; N]) -> String {
    PathBuf::from_iter(parts).display().to_string()
}

fn native_path_dir<const N: usize>(parts: [&str; N]) -> String {
    let mut path = PathBuf::from_iter(parts);
    path.push("");
    path.display().to_string()
}

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

    fn init_attachments(&self, fmt: Option<&'static str>) -> Result<()> {
        fs::create_dir_all(&self.attach_dir)?;
        if let Some(s) = fmt {
            fs::write(self.attach_dir.join(AUTOBIB_LOCKFILE), s)?;
        }
        Ok(())
    }

    fn cmd(&self) -> Result<Command> {
        self.cmd_with_attachments_dir(self.attach_dir.as_ref())
    }

    fn cmd_with_attachments_dir<P: AsRef<Path>>(&self, attachments_dir: P) -> Result<Command> {
        let mut cmd = Command::new(assert_cmd::cargo_bin!());
        cmd.arg("--database")
            .arg(self.database.as_ref())
            .arg("--config")
            .arg(self.config.as_ref())
            .arg("--attachments-dir")
            .arg(attachments_dir.as_ref())
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

    fn create_test_db(&self) -> Result<()> {
        self.cmd()?
            .args([
                "local",
                "first",
                "--with-entry-type",
                "book",
                "--with-field",
                "author = {1}",
                "--with-field",
                "title = {2}",
            ])
            .assert()
            .success();

        self.cmd()?
            .args([
                "local",
                "second",
                "--with-entry-type",
                "article",
                "--with-field",
                "author = {A}",
            ])
            .assert()
            .success();

        self.cmd()?
            .args(["edit", "local:first", "--delete-field", "author"])
            .assert()
            .success();

        self.cmd()?
            .args(["hist", "undo", "local:first"])
            .assert()
            .success();

        self.cmd()?
            .args(["edit", "local:first", "--set-field", "title = {3}"])
            .assert()
            .success();

        self.cmd()?
            .args(["hist", "undo", "local:first"])
            .assert()
            .success();

        self.cmd()?
            .args(["hist", "redo", "local:first", "0"])
            .assert()
            .success();

        self.cmd()?
            .args(["edit", "local:first", "--set-field", "title = {4}"])
            .assert()
            .success();

        self.cmd()?
            .args(["edit", "local:first", "--set-field", "title = {5}"])
            .assert()
            .success();

        self.cmd()?
            .args([
                "replace",
                "local:first",
                "--with",
                "local:second",
                "--ignore-attachments",
            ])
            .assert()
            .success();

        self.cmd()?
            .args([
                "hist",
                "revive",
                "local:first",
                "--with-field",
                "title = {6}",
            ])
            .assert()
            .success();

        self.cmd()?
            .args(["edit", "local:second", "--update-entry-type", "manuscript"])
            .assert()
            .success();

        self.cmd()?
            .args(["hist", "undo", "local:first", "--delete"])
            .assert()
            .success();

        self.cmd()?
            .args(["hist", "undo", "local:first"])
            .assert()
            .success();

        self.cmd()?
            .args(["delete", "local:first"])
            .assert()
            .success();

        self.cmd()?
            .args([
                "hist",
                "revive",
                "local:first",
                "--with-field",
                "author = {B}",
                "--with-entry-type",
                "book",
            ])
            .assert()
            .success();

        self.cmd()?
            .args(["hist", "void", "local:first"])
            .assert()
            .success();

        self.cmd()?
            .args([
                "hist",
                "revive",
                "local:first",
                "--with-field",
                "author = {C}",
                "--with-entry-type",
                "article",
            ])
            .assert()
            .success();

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

    s.cmd()?.arg("help").assert().success();

    s.close()
}

/// Check that we correctly suggest alternative keys
#[test]
fn suggest_alternatives() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?
        .args(["get", "zbl:math/0001001"])
        .assert()
        .failure()
        .stderr(contains("arxiv:math/0001001"));
    Ok(())
}

/// Check that `autobib get` returns what is expected.
#[test]
fn get() -> Result<()> {
    let s = TestState::init()?;

    let predicate_file = predicate::path::eq_file(Path::new("tests/resources/get/stdout.txt"))
        .utf8()
        .unwrap();
    s.cmd()?
        .args([
            "get",
            "zbl:1337.28015",
            "zbl:1285.28011",
            "arxiv:1212.1873",
            "mr:3224722",
        ])
        .assert()
        .success()
        .stdout(predicate_file)
        .stderr(predicate::str::is_empty());

    s.cmd()?
        .args(["--read-only", "get", "arxiv:1212.1873"])
        .assert()
        .success();

    s.cmd()?
        .args([
            "get",
            "arxiv:1212.1873",
            "zbl:1285.28011",
            "--template",
            "{pagetotal}",
            "--strict",
        ])
        .assert()
        .success()
        .stdout("368\n");

    s.cmd()?
        .args([
            "get",
            "zbl:1285.28011",
            "-t",
            r#"{author}{year}{%full_id}"#,
            "--sep",
            "$$",
        ])
        .write_stdin("arxiv:1212.1873\nmr:3224722")
        .assert()
        .success()
        .stdout("Falconer, Kenneth2014zbmath:6245248$$Hochman, Michael2014arxiv:1212.1873$$Hochman, Michael2014mr:3224722\n")
        .stderr(predicate::str::is_empty());

    s.cmd()?
        .args(["alias", "add", "my_alias", "zbmath:6245248"])
        .assert()
        .success();

    s.cmd()?
        .args([
            "get",
            "my_alias",
            "zbl:1285.28011",
            "zbmath:06245248",
            "-t",
            "{%key}",
        ])
        .assert()
        .success()
        .stdout("my_alias\nzbl:1285.28011\nzbmath:06245248\n")
        .stderr(predicate::str::is_empty());

    s.close()
}

/// Check that `autobib source --append` returns what is expected.
#[test]
fn source_append() -> Result<()> {
    let s = TestState::init()?;

    let output = NamedTempFile::new("out.bib")?;
    output.write_str("@preprint{arxiv:1212.1873,}\n")?;

    s.cmd()?
        .args([
            "source",
            "--stdin",
            "txt",
            "--out",
            &output.to_string_lossy(),
            "--append",
        ])
        .write_stdin("zbl:1337.28015\narxiv:1212.1873")
        .assert()
        .success()
        .stderr(predicate::str::is_empty());

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

    s.cmd()?
        .args(["source", "tests/resources/source/main.tex"])
        .assert()
        .success()
        .stdout(predicate_file.clone())
        .stderr(predicate::str::is_empty());

    s.cmd()?
        .args(["--read-only", "source", "tests/resources/source/main.tex"])
        .assert()
        .success()
        .stdout(predicate_file.clone())
        .stderr(predicate::str::is_empty());

    s.cmd()?
        .args(["source", "--stdin", "tex"])
        .pipe_stdin("tests/resources/source/main.tex")?
        .assert()
        .success()
        .stdout(predicate_file)
        .stderr(predicate::str::is_empty());

    let predicate_file = predicate::path::eq_file(Path::new("tests/resources/source/stdout.json"))
        .utf8()
        .unwrap();
    s.cmd()?
        .args(["source", "tests/resources/source/main.tex", "--json"])
        .assert()
        .success()
        .stdout(predicate_file)
        .stderr(predicate::str::is_empty());

    s.close()
}

/// Check that `autobib source --print-keys` works.
#[test]
fn source_keys_only() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?
        .args(["source", "tests/resources/source/main.tex", "--print-keys"])
        .assert()
        .success()
        .stdout("doi:10.4007/annals.2014.180.2.7\njfm:60.0017.02\n");

    s.close()
}

/// Check that `.typ` source detection works.
#[test]
fn source_typ() -> Result<()> {
    let s = TestState::init()?;

    let predicate_file =
        predicate::path::eq_file(Path::new("tests/resources/source_typ/stdout.txt"))
            .utf8()
            .unwrap();

    s.cmd()?
        .args([
            "source",
            "tests/resources/source_typ/main.typ",
            "--print-keys",
        ])
        .assert()
        .success()
        .stdout(predicate_file)
        .stderr(predicate::str::is_empty());

    s.close()
}

/// Check that the `--skip*` and `--append` options for `autobib source`
/// work as expected
#[test]
fn source_skip() -> Result<()> {
    let s = TestState::init()?;

    let output = NamedTempFile::new("out.bib")?;
    output.write_str("@preprint{arxiv:1212.1873,}\n")?;

    s.cmd()?
        .args([
            "source",
            "tests/resources/source_skip/main.tex",
            "--skip",
            "isbn:9781119942399",
            "--skip-from",
            "tests/resources/source_skip/skip.tex",
            "--skip-from",
            "tests/resources/source_skip/skip.bib",
            "--out",
            &output.to_string_lossy(),
            "--append",
        ])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());

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

    s.cmd()?
        .args(["get", "zbl:9999.28015"])
        .assert()
        .failure()
        .stderr(contains("Null record"));

    s.cmd()?
        .args(["get", "--ignore-null", "zbl:9999.28015"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());

    s.close()
}

/// Check that `autobib local` works as expected.
#[test]
fn local() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?
        .args([
            "local",
            "first",
            "--from-bibtex",
            "tests/resources/local/first.bib",
        ])
        .assert()
        .success();

    s.cmd()?
        .args(["--read-only", "local", "second"])
        .assert()
        .failure()
        .stderr(contains("cannot be used in read-only mode"));

    let predicate_file = predicate::path::eq_file(Path::new("tests/resources/local/stdout.txt"))
        .utf8()
        .unwrap();
    s.cmd()?
        .args(["get", "local:first"])
        .assert()
        .success()
        .stdout(predicate_file);

    s.cmd()?.args(["local", "first"]).assert().failure();

    s.cmd()?
        .args([
            "local",
            "first",
            "--from-bibtex",
            "tests/resources/local/first.bib",
        ])
        .assert()
        .failure()
        .stderr(contains("Local record 'local:first' already exists"));

    s.cmd()?.args(["local", "second"]).assert().success();

    let bibtex = NamedTempFile::new("record.bib")?;
    bibtex.write_str("@book{ignored,\n  title = {Original title},\n}\n")?;
    s.cmd()?
        .arg("local")
        .arg("override")
        .arg("--from-bibtex")
        .arg(bibtex.path())
        .args([
            "--with-entry-type",
            "article",
            "--with-field",
            "title = {Replacement title}",
        ])
        .assert()
        .success();

    s.cmd()?
        .args([
            "get",
            "local:override",
            "--template",
            "{%entry_type}|{title}",
        ])
        .assert()
        .success()
        .stdout("article|Replacement title\n");

    let predicate_file =
        predicate::path::eq_file(Path::new("tests/resources/local/stdout_short.txt"))
            .utf8()
            .unwrap();
    s.cmd()?
        .args(["get", "local:second"])
        .assert()
        .success()
        .stdout(predicate_file);

    s.cmd()?
        .args(["local", " \n"])
        .assert()
        .failure()
        .stderr(contains(
            "local sub-id must contain non-whitespace characters",
        ));

    s.cmd()?
        .args(["local", ":"])
        .assert()
        .failure()
        .stderr(contains("local sub-id must not contain a colon"));

    s.close()
}

/// Check that `autobib alias` works as expected.
#[test]
fn alias() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?
        .args([
            "local",
            "first",
            "--from-bibtex",
            "tests/resources/local/first.bib",
        ])
        .assert()
        .success();

    s.cmd()?.args(["get", "local:first"]).assert().success();

    s.cmd()?
        .args(["alias", "add", "my_alias", "local:first"])
        .assert()
        .success();

    s.cmd()?.args(["local", "second"]).assert().success();

    s.cmd()?
        .args(["alias", "add", "my_alias", "local:second"])
        .assert()
        .failure()
        .stderr(
            contains("Alias 'my_alias' already exists and refers to 'local:first'")
                .and(contains("refers to 'local:second'").not()),
        );

    let predicate_file = predicate::path::eq_file(Path::new("tests/resources/alias/stdout.txt"))
        .utf8()
        .unwrap();
    s.cmd()?
        .arg("get")
        .arg("my_alias")
        .assert()
        .success()
        .stdout(predicate_file);

    s.cmd()?
        .args(["alias", "rename", "my_alias", "new_alias"])
        .assert()
        .success();

    s.cmd()?
        .args(["get", "new_alias"])
        .assert()
        .success()
        .stdout(contains("@book{new_alias"));

    s.cmd()?
        .args(["alias", "delete", "new_alias"])
        .assert()
        .success();

    s.cmd()?
        .args(["get", "my_alias"])
        .assert()
        .failure()
        .stderr(contains("Undefined alias"));

    s.cmd()?
        .args(["get", "new_alias"])
        .assert()
        .failure()
        .stderr(contains("Undefined alias"));

    s.cmd()?
        .args(["get", "--ignore-null", "new_alias"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());

    s.cmd()?
        .args(["alias", "delete", "my_alias"])
        .assert()
        .failure()
        .stderr(contains("Could not delete alias which does not exist"));

    s.cmd()?
        .args(["alias", "add", "  ", "not_an_alias"])
        .assert()
        .failure()
        .stderr(
            contains("invalid value '  ' for '<ALIAS>'")
                .and(contains("alias must contain non-whitespace characters")),
        );

    s.cmd()?
        .args(["alias", "add", "\n\t", "not_an_alias"])
        .assert()
        .failure()
        .stderr(
            contains("invalid value '\n\t' for '<ALIAS>'")
                .and(contains("alias must contain non-whitespace characters")),
        );

    s.cmd()?
        .args(["alias", "add", "has ws", "not_an_alias"])
        .assert()
        .failure()
        .stderr(contains("Cannot create alias for undefined alias"));

    s.close()
}

/// Check that `autobib alias` works as expected with null and existing remote records.
#[test]
fn alias_remote() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?
        .args(["alias", "add", "al", "zbmath:6346461"])
        .assert()
        .success();

    let predicate_file =
        predicate::path::eq_file(Path::new("tests/resources/alias_remote/stdout.txt"))
            .utf8()
            .unwrap();
    s.cmd()?
        .args(["get", "al"])
        .assert()
        .success()
        .stdout(predicate_file);

    s.cmd()?
        .args(["alias", "add", "a2", "zbmath:96346461"])
        .assert()
        .failure()
        .stderr(contains("Cannot create alias for null record"));

    s.cmd()?
        .args(["alias", "add", "a2", "alias-does-not-exist"])
        .assert()
        .failure()
        .stderr(contains("Cannot create alias for undefined alias"));

    s.cmd()?
        .args(["get", "a2"])
        .assert()
        .failure()
        .stderr(contains("Undefined alias"));

    s.close()
}

/// Check that `autobib get` validates BibTeX citation keys and suggests alternatives on failure.
#[test]
fn bibtex_key_validation() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?
        .args([
            "alias",
            "add",
            "cst1989",
            "doi:10.1016/0021-8693(89)90256-1",
        ])
        .assert()
        .success();

    s.cmd()?
        .args(["get", "doi:10.1016/0021-8693(89)90256-1"])
        .assert()
        .failure()
        .stderr(contains("contains invalid character").and(contains("cst1989")));

    s.cmd()?.args(["get", "cst1989"]).assert().success();

    s.cmd()?
        .args([
            "get",
            "--retrieve-only",
            "doi:10.1016/0021-8693(89)90256-1",
            "cst1989",
        ])
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());

    s.cmd()?
        .args(["alias", "add", "has ws", "cst1989"])
        .assert()
        .success();

    s.cmd()?
        .args(["get", "has ws"])
        .assert()
        .failure()
        .stderr(contains("contains invalid character"));

    s.close()
}

/// Test deletion, including of aliases.
#[test]
fn delete() -> Result<()> {
    let s = TestState::init()?;

    // single deletion OK even without `--force`
    s.cmd()?.args(["get", "mr:3224722"]).assert().success();

    let attachment_dir = attachment_path(&s, "mr:3224722")?;
    fs::create_dir_all(&attachment_dir)?;
    fs::write(attachment_dir.join("attachment.txt"), "attachment contents")?;

    s.cmd()?
        .args(["delete", "mr:3224722"])
        .assert()
        .success()
        .stderr(contains("Deleted record has attachment directory"));

    s.cmd()?.args(["delete", "mr:3224722"]).assert().failure();

    // multi deletion succeeds, and applies to all aliases
    s.cmd()?
        .args([
            "local",
            "first",
            "--from-bibtex",
            "tests/resources/local/first.bib",
        ])
        .assert()
        .success();

    s.cmd()?
        .args(["alias", "add", "my_alias", "local:first"])
        .assert()
        .success();

    s.cmd()?.args(["get", "local:first"]).assert().success();

    s.cmd()?.args(["delete", "my_alias"]).assert().success();

    s.cmd()?
        .args(["get", "my_alias"])
        .assert()
        .failure()
        .stderr(contains("Deleted record"));

    s.cmd()?
        .args(["delete", "my_alias"])
        .assert()
        .failure()
        .stderr(contains("already deleted"));

    // deleting multiple
    s.cmd()?
        .args(["delete", "--hard", "local:first", "my_alias"])
        .assert()
        .failure()
        .stderr(contains("Cannot delete undefined alias"));

    s.cmd()?
        .args(["get", "local:first"])
        .assert()
        .failure()
        .stderr(contains(
            "Cannot retrieve remote data for key with local provenance",
        ));

    s.close()
}

/// Test record and citation key listing.
#[test]
fn list() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?
        .args([
            "local",
            "first",
            "--from-bibtex",
            "tests/resources/local/first.bib",
        ])
        .assert()
        .success();

    s.cmd()?
        .args(["alias", "add", "my_alias", "local:first"])
        .assert()
        .success();

    s.cmd()?.args(["get", "zbl:1337.28015"]).assert().success();

    s.cmd()?
        .args(["list"])
        .assert()
        .success()
        .stdout(contains("zbmath:6346461").and(contains("my_alias")));

    s.cmd()?
        .args(["--read-only", "list"])
        .assert()
        .success()
        .stdout(contains("zbmath:6346461").and(contains("my_alias")));

    s.cmd()?
        .args(["list", "--canonical"])
        .assert()
        .success()
        .stdout(contains("my_alias").not().and(contains("local:first")));

    s.cmd()?
        .args(["list", "local:*", "-t", "{title}"])
        .assert()
        .success()
        .stdout("My favourite book\n");

    s.cmd()?.args(["list", "--canonical", "-t", "{title}{%key}"])
    .assert().success().stdout("My favourite booklocal:first\nOn self-similar sets with overlaps and inverse theorems for entropyzbmath:6346461\n");

    s.cmd()?
        .args(["list", "*_alias", "-t", "{%key}"])
        .assert()
        .success()
        .stdout("my_alias\n");

    s.cmd()?
        .args(["list", "--canonical", "*_alias", "-t", "{%key}"])
        .assert()
        .success()
        .stdout("");

    s.cmd()?.args(["delete", "my_alias"]).assert().success();

    s.cmd()?
        .args(["list", "--deleted"])
        .assert()
        .success()
        .stdout(contains("my_alias").and(contains("local:first")));

    s.cmd()?
        .args(["list", "--deleted", "--template", "{%full_id}"])
        .assert()
        .failure();

    s.close()
}

#[test]
fn info() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?
        .args(["info", "zbl:1337.28015", "-r", "canonical"])
        .assert()
        .failure()
        .stderr(contains("Record not in database"));

    s.cmd()?.args(["get", "zbl:1337.28015"]).assert().success();

    s.cmd()?
        .args(["info", "zbl:1337.28015", "--report", "canonical"])
        .assert()
        .success()
        .stdout("zbmath:6346461\n");

    s.cmd()?
        .args([
            "--read-only",
            "info",
            "zbl:1337.28015",
            "--report",
            "canonical",
        ])
        .assert()
        .success()
        .stdout("zbmath:6346461\n");

    s.cmd()?
        .args(["alias", "add", "%", "zbmath:6346461"])
        .assert()
        .success();

    s.cmd()?
        .args(["info", "zbl:1337.28015", "-r", "equivalent"])
        .assert()
        .success()
        .stdout(
            contains("%")
                .and(contains("zbmath:6346461"))
                .and(contains("zbl:1337.28015")),
        );

    s.cmd()?
        .args(["info", "%", "-r", "valid"])
        .assert()
        .failure()
        .stderr("%\n");

    s.cmd()?
        .args(["info", "%", "--json"])
        .assert()
        .success()
        .stdout(
            contains("modified")
                .and(contains("\"canonical\":\"zbmath:6346461\""))
                .and(contains("\"is_valid_bibtex\":false"))
                .and(contains("author"))
                .and(contains("title"))
                .and(contains("\"user_preferred\":null")),
        );

    s.cmd()?.args(["info", "%"]).assert().success().stdout(
        contains("Canonical identifier: zbmath:6346461")
            .and(contains("Entry type: article"))
            .and(contains("No matching preferred key"))
            .and(contains("Key: %")),
    );

    s.close()
}

#[test]
fn attach() -> Result<()> {
    let s = TestState::init()?;

    let temp = assert_fs::NamedTempFile::new("attachment.txt")?;
    let temp_contents = "test\ncontents";
    temp.write_str(temp_contents)?;

    let attachment_file = s.attachment("zbmath/JX/TT/CT/GA3DGNBWGQ3DC===/attachment.txt");

    s.cmd()?
        .args(["attach", "zbl:1337.28015"])
        .arg(temp.as_ref())
        .assert()
        .success();

    attachment_file.assert(predicate::eq(temp_contents));

    s.cmd()?
        .args(["attach", "zbl:1337.28015"])
        .arg(temp.as_ref())
        .args(["--rename", "attach2.txt"])
        .assert()
        .success();

    s.attachment("zbmath/JX/TT/CT/GA3DGNBWGQ3DC===/attach2.txt")
        .assert(predicate::eq(temp_contents));

    s.cmd()?
        .args(["attach", "zbl:1337.28015"])
        .arg(temp.as_ref())
        .args(["--rename", "attach3.txt", "--force"])
        .assert()
        .success();

    s.attachment("zbmath/JX/TT/CT/GA3DGNBWGQ3DC===/attach3.txt")
        .assert(predicate::eq(temp_contents));

    temp.write_str("short")?;

    s.cmd()?
        .args(["attach", "zbl:1337.28015"])
        .arg(temp.as_ref())
        .arg("--force")
        .assert()
        .success();

    attachment_file.assert(predicate::eq("short"));

    s.cmd()?
        .args(["attach", "zbl:1337.28015"])
        .arg(temp.as_ref())
        .args(["--rename", ".."])
        .assert()
        .failure();

    s.cmd()?
        .args(["attach", "zbl:1337.28015"])
        .arg(temp.as_ref())
        .args(["--rename", "/invalid"])
        .assert()
        .failure();

    s.cmd()?
        .args(["attach", "zbl:1337.28015"])
        .arg(temp.as_ref())
        .args(["--rename", ""])
        .assert()
        .failure();

    s.cmd()?
        .args(["attach", "zbl:1337.28015"])
        .arg(temp.as_ref())
        .args(["--rename", "."])
        .assert()
        .failure();

    temp.close()?;
    s.close()
}

/// Check that `autobib path` always returns the same values.
#[test]
fn path_platform_consistency() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?.args(["get", "zbl:1337.28015"]).assert().success();

    let value = format!(
        "{}\n",
        native_path_dir(["zbmath", "JX", "TT", "CT", "GA3DGNBWGQ3DC==="])
    );
    s.cmd()?
        .args(["path", "zbl:1337.28015"])
        .assert()
        .success()
        .stdout(predicate::str::ends_with(value));

    s.cmd()?
        .args([
            "alias",
            "add",
            "my-alias",
            "doi:10.1016/0021-8693(89)90256-1",
        ])
        .assert()
        .success();

    let value = format!(
        "{}\n",
        native_path_dir([
            "doi",
            "XN",
            "UL",
            "PE",
            "GEYC4MJQGE3C6MBQGIYS2OBWHEZSQOBZFE4TAMRVGYWTC===",
        ])
    );

    s.cmd()?
        .args(["path", "my-alias"])
        .assert()
        .success()
        .stdout(predicate::str::ends_with(value));

    s.close()
}

fn import_zbmath_record(s: &TestState) -> Result<()> {
    fs::write(s.config.as_ref(), "preferred_keys = [\"^zbmath:.*\"]\n")?;

    s.cmd()?
        .args(["import", "tests/resources/import/file.bib"])
        .assert()
        .success();

    Ok(())
}

#[test]
fn attachment_format_missing_uses_v0() -> Result<()> {
    let s = TestState::init()?;

    import_zbmath_record(&s)?;

    let value = format!(
        "{}\n",
        native_path_dir(["zbmath", "JX", "TT", "CT", "GA3DGNBWGQ3DC==="])
    );
    s.cmd()?
        .args(["path", "zbmath:6346461"])
        .assert()
        .success()
        .stdout(predicate::str::ends_with(value));

    s.attachment(AUTOBIB_LOCKFILE).assert(predicate::eq(""));

    s.close()
}

#[test]
fn attachment_format_v1_uses_normalized_zbmath_id() -> Result<()> {
    let s = TestState::init()?;
    s.init_attachments(Some("v1"))?;

    import_zbmath_record(&s)?;

    let value = format!(
        "{}\n",
        native_path_dir(["zbmath", "6D", "UP", "TS", "GYZTINRUGYYQ"])
    );
    s.cmd()?
        .args(["path", "zbmath:6346461"])
        .assert()
        .success()
        .stdout(predicate::str::ends_with(value));

    s.close()
}

#[test]
fn attachment_format_v1_migrating_errors() -> Result<()> {
    let s = TestState::init()?;
    s.create_test_db()?;
    s.init_attachments(Some("v1-migrating"))?;

    s.cmd()?
        .args(["path", "local:first"])
        .assert()
        .failure()
        .stderr(contains("currently being migrated"));

    s.close()
}

#[test]
fn attachment_format_unknown_errors() -> Result<()> {
    let s = TestState::init()?;
    s.create_test_db()?;
    s.init_attachments(Some("v2"))?;

    s.cmd()?
        .args(["path", "local:first"])
        .assert()
        .failure()
        .stderr(contains("Attachment directory is in unknown format 'v2'"));

    s.close()
}

#[test]
fn migrate_attachments() -> Result<()> {
    let s = TestState::init()?;

    let local_old_attachment = s.attachment("local/OM/KH/CW/MZUXE43U/attachment.txt");
    local_old_attachment.write_str("local attachment contents")?;
    let zbmath_old_attachment = s.attachment("zbmath/JX/TT/CT/GA3DGNBWGQ3DC===/attachment.txt");
    zbmath_old_attachment.write_str("zbmath attachment contents")?;

    s.cmd()?
        .args(["clean", "attachments", "--migrate"])
        .assert()
        .success();

    local_old_attachment.assert(predicate::path::missing());
    s.attachment("local/QH/OV/RX/MZUXE43U/attachment.txt")
        .assert(predicate::eq("local attachment contents"));
    zbmath_old_attachment.assert(predicate::path::missing());
    s.attachment("zbmath/6D/UP/TS/GYZTINRUGYYQ/attachment.txt")
        .assert(predicate::eq("zbmath attachment contents"));
    s.attachment(AUTOBIB_LOCKFILE).assert(predicate::eq("v1"));

    s.close()
}

#[test]
fn migrate_attachments_resume() -> Result<()> {
    let s = TestState::init()?;
    s.init_attachments(Some("v1-migrating"))?;

    // `local:first` in v0
    let old_attachment = s.attachment("local/OM/KH/CW/MZUXE43U/attachment.txt");
    old_attachment.write_str("old attachment")?;

    // `local:second` in v1
    let new_attachment = s.attachment("local/EN/6D/4U/ONSWG33OMQ/attachment.txt");
    new_attachment.write_str("new attachment")?;

    s.cmd()?
        .args(["clean", "attachments", "--migrate"])
        .assert()
        .success();

    old_attachment.assert(predicate::path::missing());
    s.attachment("local/QH/OV/RX/MZUXE43U/attachment.txt")
        .assert(predicate::eq("old attachment"));
    s.attachment("local/EN/6D/4U/ONSWG33OMQ/attachment.txt")
        .assert(predicate::eq("new attachment"));
    s.attachment(AUTOBIB_LOCKFILE).assert(predicate::eq("v1"));

    s.close()
}

#[test]
fn migrate_replaces_empty_dir() -> Result<()> {
    let s = TestState::init()?;

    let old_attachment = s.attachment("local/OM/KH/CW/MZUXE43U/attachment.txt");
    old_attachment.write_str("attachment contents")?;
    fs::create_dir_all(s.attach_dir.join("local/QH/OV/RX/MZUXE43U"))?;

    s.cmd()?
        .args(["clean", "attachments", "--migrate"])
        .assert()
        .success();

    old_attachment.assert(predicate::path::missing());
    s.attachment("local/QH/OV/RX/MZUXE43U/attachment.txt")
        .assert(predicate::eq("attachment contents"));
    s.attachment(AUTOBIB_LOCKFILE).assert(predicate::eq("v1"));

    s.close()
}

#[test]
fn migrate_attachments_conflict() -> Result<()> {
    let s = TestState::init()?;

    s.attachment("local/OM/KH/CW/MZUXE43U/attachment.txt")
        .write_str("old attachment contents")?;
    s.attachment("local/QH/OV/RX/MZUXE43U/attachment.txt")
        .write_str("new attachment contents")?;
    let zbmath_old_attachment = s.attachment("zbmath/JX/TT/CT/GA3DGNBWGQ3DC===/attachment.txt");
    zbmath_old_attachment.write_str("zbmath attachment contents")?;

    s.cmd()?
        .args(["clean", "attachments", "--migrate"])
        .assert()
        .failure()
        .stderr(
            contains("Target directory already exists")
                .and(contains(native_path([
                    "local", "OM", "KH", "CW", "MZUXE43U",
                ])))
                .and(contains(native_path([
                    "local", "QH", "OV", "RX", "MZUXE43U",
                ])))
                .and(contains("Attachment migration is incomplete"))
                .and(contains("attachments --migrate")),
        );

    zbmath_old_attachment.assert(predicate::path::missing());
    s.attachment("zbmath/6D/UP/TS/GYZTINRUGYYQ/attachment.txt")
        .assert(predicate::eq("zbmath attachment contents"));
    s.attachment(AUTOBIB_LOCKFILE)
        .assert(predicate::eq("v1-migrating"));

    s.close()
}

#[test]
fn migrate_attachments_unrecognized() -> Result<()> {
    let s = TestState::init()?;

    s.attachment("local/not/base32/path/not-base32")
        .write_str("ignored contents")?;
    s.attachment("local/AA/AA/AA/not-base32/attachment.txt")
        .write_str("ignored contents")?;
    s.attachment("unknown/AA/AA/AA/MZUXE43U/attachment.txt")
        .write_str("ignored contents")?;
    s.attachment("local/AA/AA/AA/MZUXE43U/attachment.txt")
        .write_str("ignored contents")?;

    let old_attachment = s.attachment("local/OM/KH/CW/MZUXE43U/attachment.txt");
    old_attachment.write_str("attachment contents")?;

    s.cmd()?
        .args(["clean", "attachments", "--migrate"])
        .assert()
        .success()
        .stderr(
            contains("Skipping invalid attachment directory")
                .and(contains("not-base32"))
                .and(contains(native_path([
                    "unknown", "AA", "AA", "AA", "MZUXE43U",
                ])))
                .and(contains(native_path([
                    "local", "AA", "AA", "AA", "MZUXE43U",
                ]))),
        );

    s.attachment("local/not/base32/path/not-base32")
        .assert(predicate::eq("ignored contents"));
    s.attachment("local/AA/AA/AA/not-base32")
        .assert(predicate::path::is_dir());
    s.attachment("unknown/AA/AA/AA/MZUXE43U")
        .assert(predicate::path::is_dir());
    s.attachment("local/AA/AA/AA/MZUXE43U")
        .assert(predicate::path::is_dir());
    old_attachment.assert(predicate::path::missing());
    s.attachment("local/QH/OV/RX/MZUXE43U/attachment.txt")
        .assert(predicate::eq("attachment contents"));

    s.close()
}

#[test]
fn edit() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?
        .args(["edit", "zbl:9999.28015"])
        .assert()
        .failure()
        .stderr(contains("Cannot edit null record"));

    s.cmd()?
        .args(["edit", "my_alias"])
        .assert()
        .failure()
        .stderr(contains("Cannot edit undefined alias"));

    let predicate_file =
        predicate::path::eq_file(Path::new("tests/resources/edit/stdout_unedited.txt"))
            .utf8()
            .unwrap();
    s.cmd()?
        .args(["get", "mr:3224722"])
        .assert()
        .success()
        .stdout(predicate_file);

    s.cmd()?
        .args(["edit", "--set-eprint=zbl,doi", "mr:3224722"])
        .assert()
        .success();

    s.cmd()?
        .args([
            "edit",
            "mr:3224722",
            "--normalize-whitespace",
            "--set-field",
            "note = {Expected note}",
        ])
        .assert()
        .success();

    let predicate_file = predicate::path::eq_file(Path::new("tests/resources/edit/stdout.txt"))
        .utf8()
        .unwrap();
    s.cmd()?
        .args(["get", "mr:3224722"])
        .assert()
        .success()
        .stdout(predicate_file);

    s.close()
}

#[test]
fn update() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?
        .args(["update", "zbmath:6346461"])
        .assert()
        .failure()
        .stderr(contains("does not exist in database").and(contains("Use `autobib get`")));

    s.close()
}

#[test]
fn update_local() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?.args(["local", "one"]).assert().success();

    s.cmd()?.args(["get", "local:one"]).assert().success();

    s.cmd()?
        .args(["update", "local:one"])
        .assert()
        .failure()
        .stderr(contains("Cannot update local record using remote data"));

    s.cmd()?
        .args(["update", "local:two"])
        .assert()
        .failure()
        .stderr(contains("does not exist in database").and(contains("Use `autobib get`").not()));

    s.close()
}

#[test]
fn consistency() -> Result<()> {
    use rusqlite::Connection;

    let s = TestState::init()?;

    s.cmd()?
        .args([
            "get",
            "--retrieve-only",
            "zbmath:6346461",
            "zbl:1337.28015",
            "mr:3224722",
        ])
        .assert()
        .success();

    // Simulate a failed data migration by leaving most records in v1 while
    // replacing one with unsorted v0 data.
    let failed_migration_data = b"\0\x07article\x05\x05\0titleTitle\x06\x06\0authorAuthor";

    // Perform some destructive changes to the database.
    let conn = Connection::open(s.database.path())?;
    conn.pragma_update(None, "foreign_keys", 0)?;
    conn.prepare(
        "UPDATE Records SET data = ?1
        WHERE rev = (SELECT record_rev FROM Keys WHERE name = 'mr:3224722')",
    )?
    .execute([failed_migration_data.as_slice()])?;
    conn.prepare(
        "INSERT INTO Keys (name, record_rev) VALUES
            ('zbmath:06346461', (SELECT record_rev FROM Keys WHERE name = 'zbmath:6346461')),
            ('zbmath:096346461', (SELECT record_rev FROM Keys WHERE name = 'zbl:1337.28015')),
            ('local:dangling', 1000000)",
    )?
    .execute(())?;
    drop(conn);

    // Check that the failed migration and key faults are detected.
    s.cmd()?.args(["util", "check"]).assert().failure().stderr(
        contains("record id 'mr:3224722' has malformed binary data")
            .and(contains(
                "Keys table contains key 'zbmath:06346461' which is not normalized",
            ))
            .and(contains(
                "Keys table contains key 'zbmath:096346461' which is not normalized",
            ))
            .and(contains(
                "An identifier references a record which does not exist",
            )),
    );

    // Repair the major recoverable routes.
    s.cmd()?
        .args(["util", "check", "--fix"])
        .assert()
        .success()
        .stderr(
            contains("Deleting identifiers which do not reference records")
                .and(contains("Deleting non-normalized key 'zbmath:06346461'"))
                .and(contains(
                    "Normalizing key 'zbmath:096346461' to 'zbmath:96346461'",
                )),
        );

    s.cmd()?.args(["util", "check"]).assert().success();
    s.cmd()?
        .args(["get", "--retrieve-only", "mr:3224722", "zbmath:96346461"])
        .assert()
        .success();

    s.close()
}

/// Check that `autobib source` warns if there are multiple references to the same key
#[test]
fn repeat() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?
        .args(["source", "--stdin", "txt"])
        .write_stdin("zbmath:6346461\nzbl:1337.28015")
        .assert()
        .success()
        .stderr(contains("Multiple keys for "));

    s.cmd()?
        .args(["alias", "add", "a", "zbl:1337.28015"])
        .assert()
        .success();

    s.cmd()?
        .args(["source", "--stdin", "txt"])
        .write_stdin("zbmath:6346461\na")
        .assert()
        .success()
        .stderr(contains("Multiple keys for "));

    s.close()
}

#[test]
fn config() -> Result<()> {
    let s = TestState::init()?;

    s.set_config(Path::new("tests/resources/config/malformed.toml"))?;
    s.cmd()?.arg("get").assert().failure();

    s.set_config(Path::new("tests/resources/config/extra.toml"))?;
    s.cmd()?.arg("get").assert().failure();

    s.set_config(Path::new("tests/resources/config/invalid_alias_rules.toml"))?;
    s.cmd()?.args(["get", "alias"]).assert().failure().stderr(
        contains("failed to compile 'alias_transform.rules' transformation")
            .and(contains("regex does not contain any capture groups"))
            .and(contains("panicked").not()),
    );

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
    s.cmd()?
        .args(["get", "mr:3224722"])
        .assert()
        .success()
        .stdout(predicate_file);

    s.close()
}

/// Test identifiers which have previously caused errors
#[test]
fn identifier_exceptions() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?
        .args(["get", "arxiv:2112.04570"])
        .assert()
        .success();

    s.close()
}

#[test]
fn quiet_returns_error() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?
        .args(["-q", "-q", "get", "::invalid"])
        .assert()
        .failure();

    s.close()
}

#[test]
fn cache_evict() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?.args(["get", "zbmath:96346461"]).assert().failure();

    s.cmd()?
        .args(["-v", "clean", "database", "--evict", "10000"])
        .assert()
        .success()
        .stderr(contains("Removed 0 cached null"));

    s.cmd()?
        .args(["-v", "clean", "database", "--evict-all"])
        .assert()
        .success()
        .stderr(contains("Removed 1 cached null"));

    s.close()
}

#[test]
fn normalize() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?
        .args(["get", "zbmath:6346461"])
        .assert()
        .success()
        .stdout(
            predicate::path::eq_file(Path::new("tests/resources/normalize/stdout.txt"))
                .utf8()
                .unwrap(),
        );

    s.cmd()?
        .args(["info", "zbmath:01111111"])
        .assert()
        .failure()
        .stderr(contains("converted from 'zbmath:01111111'"));

    s.cmd()?
        .args(["info", "zbmath:00000000"])
        .assert()
        .failure()
        .stderr(contains("converted from 'zbmath:00000000'"));

    s.close()
}

#[test]
fn strip_journal_series() -> Result<()> {
    let s = TestState::init()?;

    s.set_config(Path::new(
        "tests/resources/strip_journal_series/config.toml",
    ))?;

    s.cmd()?
        .args(["get", "zbl:1337.28015"])
        .assert()
        .success()
        .stdout(
            predicate::path::eq_file(Path::new("tests/resources/strip_journal_series/stdout.txt"))
                .utf8()
                .unwrap(),
        );

    s.close()
}

#[test]
fn auto_alias() -> Result<()> {
    let s = TestState::init()?;

    s.set_config(Path::new("tests/resources/auto_alias/config.toml"))?;

    s.cmd()?
        .args(["get", "zbMATH06346461"])
        .assert()
        .success()
        .stdout(
            predicate::path::eq_file(Path::new("tests/resources/auto_alias/stdout.txt"))
                .utf8()
                .unwrap(),
        );

    s.cmd()?
        .args(["get", "zbMATH6346461"])
        .assert()
        .failure()
        .stderr(contains("Undefined alias"));

    s.cmd()?.args(["get", "zbl:1337.28015"]).assert().success();

    s.cmd()?
        .args(["info", "zbl:1337.28015", "--report", "equivalent"])
        .assert()
        .success()
        .stdout(contains("zbMATH06346461"));

    s.cmd()?.args(["get", "mr:3224722"]).assert().success();

    s.cmd()?
        .args(["get", "MR3224722"])
        .assert()
        .success()
        .stdout(
            predicate::path::eq_file(Path::new("tests/resources/auto_alias/stdout_mr.txt"))
                .utf8()
                .unwrap(),
        );

    s.cmd()?
        .args(["info", "MR3224722", "--report", "equivalent"])
        .assert()
        .success()
        .stdout(contains("mr:3224722"));

    s.close()
}

#[test]
fn import_basic() -> Result<()> {
    let s = TestState::init()?;

    s.set_config("tests/resources/import/config.toml")?;

    s.cmd()?
        .args(["import", "tests/resources/import/file.bib"])
        .assert()
        .success();

    s.cmd()?
        .args(["get", "attainable-assouad-spectra"])
        .assert()
        .success()
        .stdout(
            predicate::path::eq_file(Path::new("tests/resources/import/stdout_local.txt"))
                .utf8()
                .unwrap(),
        );

    s.cmd()?
        .args(["get", "zbmath:6346461"])
        .assert()
        .success()
        .stdout(contains("doi = {10.4007/annals.2014.180.2.7}"));

    let bibtex = NamedTempFile::new("canonical-key.bib")?;
    bibtex.write_str(
        "@article{doi:10.1000/test,\n  title = {Canonical identifier from entry key},\n}",
    )?;

    s.cmd()?.arg("import").arg(bibtex.path()).assert().success();

    s.cmd()?
        .args(["info", "doi:10.1000/test", "--report", "canonical"])
        .assert()
        .success()
        .stdout("doi:10.1000/test\n");

    s.close()
}

#[test]
fn import_idempotent() -> Result<()> {
    let s = TestState::init()?;
    s.set_config("tests/resources/import/config.toml")?;

    s.cmd()?
        .args(["import", "tests/resources/import/file.bib"])
        .assert()
        .success();

    s.cmd()?
        .args(["import", "tests/resources/import/file.bib"])
        .assert()
        .success();

    s.cmd()?
        .args(["import", "tests/resources/import/file.bib"])
        .assert()
        .success();

    s.cmd()?
        .args(["hist", "undo", "attainable-assouad-spectra"])
        .assert()
        .failure()
        .stderr(contains("Cannot void record"));

    s.cmd()?
        .args(["hist", "undo", "zbMATH06346461"])
        .assert()
        .failure()
        .stderr(contains("Cannot void record"));

    s.close()
}

#[test]
fn import_no_alias() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?
        .args(["import", "tests/resources/import/file.bib", "--no-alias"])
        .assert()
        .success();

    s.cmd()?
        .args(["get", "attainable-assouad-spectra"])
        .assert()
        .failure()
        .stderr(contains("Undefined alias"));

    s.close()
}

#[test]
fn no_key() -> Result<()> {
    let s = TestState::init()?;

    // no key for the import
    s.cmd()?
        .args(["import", "tests/resources/import/no_ids.bib"])
        .assert()
        .failure()
        .stdout(contains("Could not determine candidate key"));

    // but succeeds with local fallback
    s.cmd()?
        .args([
            "import",
            "tests/resources/import/no_ids.bib",
            "--local-fallback",
        ])
        .assert()
        .success();

    s.cmd()?
        .args(["get", "local:my-article"])
        .assert()
        .success()
        .stdout(contains("John, Doe"));

    s.cmd()?
        .args([
            "import",
            "tests/resources/import/file.bib",
            "--local-fallback",
        ])
        .assert()
        .success();

    // local fallback is not used if the key could be determined
    s.cmd()?
        .args(["get", "local:zbMATH06346461"])
        .assert()
        .failure();

    // local fallback is not used if reference key is found
    s.cmd()?
        .args([
            "import",
            "tests/resources/import/retrieve.bib",
            "--local-fallback",
        ])
        .assert()
        .failure()
        .stdout(contains("Failed to determine canonical id"));

    s.close()
}

#[test]
fn import_local_fallback_fails() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?.args(["local", "my-article"]).assert().success();

    // local key already exists
    s.cmd()?
        .args([
            "import",
            "tests/resources/import/no_ids.bib",
            "--local-fallback",
        ])
        .assert()
        .failure()
        .stdout(contains("Local id 'local:my-article' already exists"));

    // contains colon
    s.cmd()?
        .args([
            "import",
            "tests/resources/import/id_contains_colon.bib",
            "--local-fallback",
        ])
        .assert()
        .failure()
        .stdout(contains("provider is invalid"));

    s.close()
}

#[test]
fn import_retrieve() -> Result<()> {
    let s = TestState::init()?;

    s.set_config("tests/resources/import/config.toml")?;

    s.cmd()?
        .args(["import", "tests/resources/import/retrieve.bib"])
        .assert()
        .failure()
        .stdout(contains("Failed to determine canonical id"));

    s.cmd()?
        .args(["import", "tests/resources/import/retrieve.bib", "--resolve"])
        .assert()
        .success();

    s.cmd()?
        .args(["info", "abc", "--report", "canonical"])
        .assert()
        .success()
        .stdout("zbmath:6346461\n");

    s.close()
}

#[test]
fn import_update() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?.args(["get", "zbmath:6346461"]).assert().success();

    s.cmd()?
        .args([
            "import",
            "tests/resources/import/retrieve.bib",
            "--resolve",
            "--update",
            "prefer-current",
        ])
        .assert()
        .success();

    s.cmd()?
        .args(["get", "zbmath:6346461"])
        .assert()
        .success()
        .stdout(contains("note = {extra}").and(contains("inverse theorems for entropy")));

    s.cmd()?
        .args([
            "import",
            "tests/resources/import/retrieve.bib",
            "--resolve",
            "--update",
            "prefer-incoming",
        ])
        .assert()
        .success();

    s.cmd()?
        .args(["get", "zbmath:6346461"])
        .assert()
        .success()
        .stdout(
            contains("@book{")
                .and(contains("note = {extra}"))
                .and(contains("overlaps and typos")),
        );

    s.close()
}

#[test]
fn read_only() -> Result<()> {
    let s = TestState::init()?;

    s.cmd()?.args(["get", "zbl:1337.28015"]).assert().success();

    s.cmd()?
        .args(["--read-only", "get", "zbl:1337.28015"])
        .assert()
        .success();

    s.cmd()?
        .args(["--read-only", "get", "arxiv:1212.1873"])
        .assert()
        .failure()
        .stderr(contains("Database does not contain key"));

    s.cmd()?
        .args(["--read-only", "util", "check"])
        .assert()
        .success();

    s.cmd()?
        .args(["--read-only", "info", "zbl:1337.28015"])
        .assert()
        .success();

    s.cmd()?
        .args(["--read-only", "info", "arxiv:1212.1873"])
        .assert()
        .failure()
        .stderr(contains("Record not in database"));

    s.attachment(AUTOBIB_LOCKFILE)
        .assert(predicate::path::missing());

    s.cmd()?
        .args(["--read-only", "path", "zbl:1337.28015"])
        .assert()
        .success();

    s.attachment(AUTOBIB_LOCKFILE)
        .assert(predicate::path::missing());

    s.cmd()?.args(["path", "zbl:1337.28015"]).assert().success();

    s.attachment(AUTOBIB_LOCKFILE).assert(predicate::eq(""));

    Ok(())
}

#[test]
fn replace_auto() -> Result<()> {
    let s = TestState::init()?;

    s.set_config("tests/resources/import/config.toml")?;

    s.cmd()?
        .args(["import", "tests/resources/import/init.bib", "--resolve"])
        .assert()
        .success();

    s.cmd()?
        .args(["replace", "arxiv:1212.1873", "--auto"])
        .assert()
        .failure()
        .stderr(contains("is equivalent to the current identifier"));

    s.cmd()?
        .args(["alias", "add", "arx", "arxiv:1212.1873"])
        .assert()
        .success();

    s.cmd()?
        .args([
            "edit",
            "arxiv:1212.1873",
            "--set-field",
            "zbmath = {6346461}",
        ])
        .assert()
        .success();

    s.cmd()?
        .args(["replace", "arxiv:1212.1873", "--auto"])
        .assert()
        .success();

    s.cmd()?
        .args(["get", "arxiv:1212.1873"])
        .assert()
        .failure()
        .stderr(contains(
            "Perhaps use the replacement key: 'zbmath:6346461'",
        ));

    s.cmd()?
        .args(["get", "arx"])
        .assert()
        .failure()
        .stderr(contains(
            "Perhaps use the replacement key: 'zbmath:6346461'",
        ));

    s.cmd()?
        .args(["get", "zbmath:6346461"])
        .assert()
        .success()
        .stdout(contains("@article{"));

    s.close()
}

fn create_replace_records(s: &TestState) -> Result<()> {
    s.cmd()?
        .args([
            "local",
            "first",
            "--with-entry-type",
            "book",
            "--with-field",
            "title = {First}",
        ])
        .assert()
        .success();

    s.cmd()?
        .args([
            "local",
            "second",
            "--with-entry-type",
            "book",
            "--with-field",
            "title = {Second}",
        ])
        .assert()
        .success();

    Ok(())
}

fn attachment_path(s: &TestState, id: &str) -> Result<PathBuf> {
    let output = s
        .cmd()?
        .args(["path", id])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    Ok(PathBuf::from(String::from_utf8(output)?.trim()))
}

#[test]
fn disallowed_bibtex() -> Result<()> {
    let s = TestState::init()?;
    s.cmd()?
        .args(["local", "first", "--with-entry-type", "comment"])
        .assert()
        .failure()
        .stderr(contains("reserved"));

    s.cmd()?
        .args(["local", "first", "--with-entry-type", "StrinG"])
        .assert()
        .failure()
        .stderr(contains("reserved"));

    s.cmd()?
        .args(["local", "first", "--with-entry-type", &"a".repeat(255)])
        .assert()
        .success();

    s.cmd()?
        .args(["local", "second", "--with-entry-type", " a"])
        .assert()
        .failure()
        .stderr(contains("contains invalid character"));

    s.close()
}

#[test]
fn replace_migrates_attachments() -> Result<()> {
    let s = TestState::init()?;
    create_replace_records(&s)?;

    s.cmd()?
        .args(["replace", "local:first", "--with", "local:second"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    s.close()?;

    let s = TestState::init()?;
    create_replace_records(&s)?;
    let source = attachment_path(&s, "local:first")?;
    let target = attachment_path(&s, "local:second")?;
    fs::create_dir_all(&source)?;
    fs::write(source.join("attachment.txt"), "source attachment")?;

    s.cmd()?
        .args(["replace", "local:first", "--with", "local:second"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    assert!(!source.exists());
    assert_eq!(
        fs::read_to_string(target.join("attachment.txt"))?,
        "source attachment"
    );
    s.close()?;

    let s = TestState::init()?;
    create_replace_records(&s)?;
    let source = attachment_path(&s, "local:first")?;
    let target = attachment_path(&s, "local:second")?;
    fs::create_dir_all(&target)?;
    fs::write(target.join("attachment.txt"), "target attachment")?;

    s.cmd()?
        .args(["replace", "local:first", "--with", "local:second"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    assert!(!source.exists());
    assert_eq!(
        fs::read_to_string(target.join("attachment.txt"))?,
        "target attachment"
    );
    s.close()?;

    let s = TestState::init()?;
    create_replace_records(&s)?;
    let source = attachment_path(&s, "local:first")?;
    let target = attachment_path(&s, "local:second")?;
    fs::create_dir_all(&source)?;
    fs::write(source.join("attachment.txt"), "source attachment")?;
    fs::create_dir_all(&target)?;
    fs::write(target.join("attachment.txt"), "target attachment")?;

    s.cmd()?
        .args(["replace", "local:first", "--with", "local:second"])
        .assert()
        .success()
        .stderr(
            contains("Could not merge attachment directories").and(contains(
                "Move attachment files from the original directory",
            )),
        );
    assert_eq!(
        fs::read_to_string(source.join("attachment.txt"))?,
        "source attachment"
    );
    assert_eq!(
        fs::read_to_string(target.join("attachment.txt"))?,
        "target attachment"
    );

    s.close()
}

#[test]
fn replace_hard() -> Result<()> {
    let s = TestState::init()?;

    s.set_config("tests/resources/import/config.toml")?;

    s.cmd()?
        .args(["import", "tests/resources/import/init.bib", "--resolve"])
        .assert()
        .success();

    s.cmd()?
        .args(["replace", "arxiv:1212.1873", "--auto", "--hard"])
        .assert()
        .failure()
        .stderr(contains("is equivalent to the current identifier"));

    s.cmd()?
        .args(["alias", "add", "arx", "arxiv:1212.1873"])
        .assert()
        .success();

    s.cmd()?
        .args([
            "edit",
            "arxiv:1212.1873",
            "--set-field",
            "zbmath = {6346461}",
        ])
        .assert()
        .success();

    s.cmd()?
        .args(["replace", "arxiv:1212.1873", "--auto", "--hard"])
        .assert()
        .success();

    s.cmd()?
        .args(["get", "arx"])
        .assert()
        .success()
        .stdout(contains("zbmath = {6346461}"));

    s.close()
}

#[test]
fn json_schema() -> Result<()> {
    fn assert_schema<const N: usize>(
        s: &TestState,
        args: [&'static str; N],
        schema_path: &str,
    ) -> Result<()> {
        let mut schemas = Schemas::new();
        let mut compiler = Compiler::new();
        let sch_index = compiler.compile(schema_path, &mut schemas)?;
        let instance: serde_json::Value =
            serde_json::from_slice(&s.cmd()?.args(args).assert().success().get_output().stdout)?;
        assert!(schemas.validate(&instance, sch_index).is_ok());

        Ok(())
    }

    use boon::{Compiler, Schemas};
    let s = TestState::init()?;

    assert_schema(
        &s,
        ["source", "tests/resources/source/main.tex", "--json"],
        "docs/schema/source.schema.json",
    )?;

    s.cmd()?
        .args([
            "get",
            "zbl:1337.28015",
            "zbl:1285.28011",
            "arxiv:1212.1873",
            "mr:3224722",
        ])
        .assert()
        .success();

    for id in [
        "zbl:1337.28015",
        "zbl:1285.28011",
        "arxiv:1212.1873",
        "mr:3224722",
    ] {
        assert_schema(
            &s,
            ["get", id, "-t", "{%json}"],
            "docs/schema/record_entry.schema.json",
        )?;
    }

    s.cmd()?
        .args(["delete", "zbl:1285.28011"])
        .assert()
        .success();

    s.cmd()?
        .args(["replace", "arxiv:1212.1873", "--with", "mr:3224722"])
        .assert()
        .success();

    for id in [
        "zbl:1337.28015",
        "zbl:1285.28011",
        "arxiv:1212.1873",
        "mr:3224722",
    ] {
        assert_schema(&s, ["info", "--json", id], "docs/schema/info.schema.json")?;
    }

    s.cmd()?
        .args(["hist", "void", "mr:3224722"])
        .assert()
        .success();
    assert_schema(
        &s,
        ["info", "--json", "mr:3224722"],
        "docs/schema/info.schema.json",
    )?;

    s.close()
}

#[test]
fn changelog() -> Result<()> {
    let s = TestState::init()?;
    s.create_test_db()?;

    s.cmd()?
        .args(["log", "local:first", "--all"])
        .assert()
        .success()
        .stdout(contains("Void"));

    s.cmd()?
        .args(["hist", "reset", "local:first", "0006"])
        .assert()
        .success();

    s.cmd()?
        .args(["get", "local:first"])
        .assert()
        .success()
        .stdout(contains("title = {5}"));

    s.cmd()?
        .args(["log", "local:first"])
        .assert()
        .success()
        .stdout(
            contains("│      title = {4}")
                .and(contains("◉  rev 0006"))
                .and(contains("Void").not()),
        );

    s.cmd()?
        .args(["log", "local:first", "--tree", "--all"])
        .assert()
        .success()
        .stdout(
            contains("│ │ │    }")
                .and(contains("├─╯ │"))
                .and(contains("○ │ │  rev 0009 on"))
                .and(contains(
                    "│ │ │    Replaced 'local:first' with 'local:second'",
                ))
                .and(contains("│      author = {C},")),
        );

    s.cmd()?
        .args(["hist", "reset", "local:first", "000c"])
        .assert()
        .success();

    s.cmd()?
        .args(["hist", "undo", "local:first"])
        .assert()
        .failure()
        .stderr(contains(
            "suggestion: Undo into a deleted state with `autobib hist undo --delete`",
        ));

    s.close()
}

#[test]
fn void_visibility() -> Result<()> {
    let s = TestState::init()?;
    s.create_test_db()?;

    s.cmd()?
        .args(["hist", "show"])
        .assert()
        .success()
        .stdout(contains("local:first").and(contains("Void").not()));

    s.cmd()?
        .args(["log", "local:first"])
        .assert()
        .success()
        .stdout(contains("Void").not());

    s.cmd()?
        .args(["log", "local:first", "--all"])
        .assert()
        .success()
        .stdout(contains("Void"));

    s.close()
}

#[test]
fn prune() -> Result<()> {
    fn init() -> Result<(TestState, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)> {
        let s = TestState::init()?;

        // create a node with two children
        s.cmd()?
            .args(["local", "a", "--with-field", "title = {1}"])
            .assert()
            .success();

        s.cmd()?
            .args(["edit", "local:a", "--set-field", "title = {2}"])
            .assert()
            .success();

        let output = s
            .cmd()?
            .args(["info", "local:a", "-r", "revision"])
            .output()?;
        assert!(output.status.success());
        let rev_1 = output.stdout;

        s.cmd()?
            .args(["hist", "undo", "local:a"])
            .assert()
            .success();

        s.cmd()?
            .args(["edit", "local:a", "--set-field", "title = {3}"])
            .assert()
            .success();

        // create a node with two children, and then that child has the active node
        s.cmd()?
            .args(["local", "b", "--with-field", "title = {1}"])
            .assert()
            .success();

        s.cmd()?
            .args(["edit", "local:b", "--set-field", "title = {2}"])
            .assert()
            .success();

        let output = s
            .cmd()?
            .args(["info", "local:b", "-r", "revision"])
            .output()?;
        assert!(output.status.success());
        let rev_2 = output.stdout;

        s.cmd()?
            .args(["hist", "undo", "local:b"])
            .assert()
            .success();

        s.cmd()?
            .args(["edit", "local:b", "--set-field", "title = {3}"])
            .assert()
            .success();

        s.cmd()?
            .args(["edit", "local:b", "--set-field", "title = {4}"])
            .assert()
            .success();

        s.cmd()?
            .args(["edit", "local:b", "--set-field", "title = {5}"])
            .assert()
            .success();

        let output = s
            .cmd()?
            .args(["info", "local:b", "-r", "revision"])
            .output()?;
        assert!(output.status.success());
        let rev_3 = output.stdout;

        s.cmd()?.args(["delete", "local:b"]).assert().success();

        let output = s
            .cmd()?
            .args(["info", "local:b", "-r", "revision"])
            .output()?;
        assert!(output.status.success());
        let rev_4 = output.stdout;

        s.cmd()?
            .args(["hist", "undo", "local:b"])
            .assert()
            .success();

        s.cmd()?
            .args(["hist", "undo", "local:b"])
            .assert()
            .success();

        Ok((s, rev_1, rev_2, rev_3, rev_4))
    }

    // pruning all deletes past and present states
    let (s, rev_1, rev_2, rev_3, _) = init()?;

    s.cmd()?.args(["hist", "prune", "all"]).assert().success();

    s.cmd()?
        .args([
            "hist",
            "reset",
            "local:a",
            std::str::from_utf8(&rev_1).unwrap().trim_end(),
        ])
        .assert()
        .failure()
        .stderr(contains("Revision does not exist"));

    s.cmd()?
        .args([
            "hist",
            "reset",
            "local:b",
            std::str::from_utf8(&rev_2).unwrap().trim_end(),
        ])
        .assert()
        .failure()
        .stderr(contains("Revision does not exist"));

    s.cmd()?
        .args([
            "hist",
            "reset",
            "local:b",
            std::str::from_utf8(&rev_3).unwrap().trim_end(),
        ])
        .assert()
        .failure()
        .stderr(contains("Revision does not exist"));

    // pruning outdated does not delete future states
    let (s, rev_1, rev_2, rev_3, _) = init()?;

    s.cmd()?
        .args(["hist", "prune", "outdated"])
        .assert()
        .success();

    s.cmd()?
        .args([
            "hist",
            "reset",
            "local:a",
            std::str::from_utf8(&rev_1).unwrap().trim_end(),
        ])
        .assert()
        .failure()
        .stderr(contains("Revision does not exist"));

    s.cmd()?
        .args([
            "hist",
            "reset",
            "local:b",
            std::str::from_utf8(&rev_2).unwrap().trim_end(),
        ])
        .assert()
        .failure()
        .stderr(contains("Revision does not exist"));

    s.cmd()?
        .args([
            "hist",
            "reset",
            "local:b",
            std::str::from_utf8(&rev_3).unwrap().trim_end(),
        ])
        .assert()
        .success();

    s.close()?;

    // keep the correct number
    let (s, rev_1, rev_2, rev_3, _) = init()?;

    s.cmd()?
        .args(["hist", "prune", "outdated", "--retain", "1"])
        .assert()
        .success();

    s.cmd()?
        .args([
            "hist",
            "reset",
            "local:a",
            std::str::from_utf8(&rev_1).unwrap().trim_end(),
        ])
        .assert()
        .success();

    s.cmd()?
        .args([
            "hist",
            "reset",
            "local:b",
            std::str::from_utf8(&rev_2).unwrap().trim_end(),
        ])
        .assert()
        .failure()
        .stderr(contains("Revision does not exist"));

    s.cmd()?
        .args([
            "hist",
            "reset",
            "local:b",
            std::str::from_utf8(&rev_3).unwrap().trim_end(),
        ])
        .assert()
        .success();

    s.close()?;

    // keep the correct number
    let (s, rev_1, rev_2, rev_3, _) = init()?;

    s.cmd()?
        .args(["hist", "prune", "outdated", "--retain", "2"])
        .assert()
        .success();

    s.cmd()?
        .args([
            "hist",
            "reset",
            "local:a",
            std::str::from_utf8(&rev_1).unwrap().trim_end(),
        ])
        .assert()
        .success();

    s.cmd()?
        .args([
            "hist",
            "reset",
            "local:b",
            std::str::from_utf8(&rev_2).unwrap().trim_end(),
        ])
        .assert()
        .success();

    s.cmd()?
        .args([
            "hist",
            "reset",
            "local:b",
            std::str::from_utf8(&rev_3).unwrap().trim_end(),
        ])
        .assert()
        .success();

    s.close()?;

    // prune deleted
    let (s, rev_1, rev_2, rev_3, rev_4) = init()?;

    s.cmd()?
        .args(["hist", "prune", "deleted"])
        .assert()
        .success();

    s.cmd()?
        .args([
            "hist",
            "reset",
            "local:a",
            std::str::from_utf8(&rev_1).unwrap().trim_end(),
        ])
        .assert()
        .success();

    s.cmd()?
        .args([
            "hist",
            "reset",
            "local:b",
            std::str::from_utf8(&rev_2).unwrap().trim_end(),
        ])
        .assert()
        .success();

    s.cmd()?
        .args([
            "hist",
            "reset",
            "local:b",
            std::str::from_utf8(&rev_3).unwrap().trim_end(),
        ])
        .assert()
        .success();

    s.cmd()?
        .args([
            "hist",
            "reset",
            "local:b",
            std::str::from_utf8(&rev_4).unwrap().trim_end(),
        ])
        .assert()
        .stderr(contains("Revision does not exist"));

    s.close()?;

    Ok(())
}

macro_rules! test_provider_success {
    ($name:ident, $target:expr) => {
        /// Check that `autobib get` succeeds
        #[test]
        fn $name() -> Result<()> {
            let s = TestState::init()?;

            s.cmd()?.args(["-vv", "get", $target]).assert().success();

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
