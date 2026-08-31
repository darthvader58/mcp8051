//! `FIRMWARE_ROOT` confinement, including the escapes that a textual prefix
//! check would wave through.

mod common;

use common::TempDir;
use mcs51_mcp::errors::{AppError, ErrorCode};
use mcs51_mcp::paths::PathResolver;

fn code(err: AppError) -> ErrorCode {
    err.code()
}

#[test]
fn a_file_inside_the_root_resolves() {
    let root = TempDir::new("inside");
    let src = root.write("firmware.c", "void main(){}\n");

    let r = PathResolver::new(Some(root.path().to_path_buf()));
    assert_eq!(r.resolve_input("firmware.c").unwrap(), src);
    assert_eq!(r.resolve_input(src.to_str().unwrap()).unwrap(), src);
    assert_eq!(r.confinement(), "on");
}

#[test]
fn relative_paths_resolve_against_the_root_not_the_cwd() {
    let root = TempDir::new("relbase");
    root.dir("sub");
    let nested = root.write("sub/blink.c", "void main(){}\n");

    let r = PathResolver::new(Some(root.path().to_path_buf()));
    assert_eq!(r.resolve_input("sub/blink.c").unwrap(), nested);
}

#[test]
fn dot_dot_cannot_climb_out() {
    let root = TempDir::new("dotdot");
    let outside = TempDir::new("dotdot-outside");
    outside.write("secret.c", "nope\n");
    root.dir("sub");

    let r = PathResolver::new(Some(root.path().to_path_buf()));

    // Straight up and over.
    let err = r
        .resolve_input(&format!(
            "../{}/secret.c",
            outside.path().file_name().unwrap().to_string_lossy()
        ))
        .expect_err("`..` must not escape the root");
    assert_eq!(code(err), ErrorCode::PathEscapesFirmwareRoot);

    // And the same trick buried mid-path, where a naive check might miss it.
    let err = r
        .resolve_input(&format!(
            "sub/../../{}/secret.c",
            outside.path().file_name().unwrap().to_string_lossy()
        ))
        .expect_err("`..` in the middle must not escape either");
    assert_eq!(code(err), ErrorCode::PathEscapesFirmwareRoot);
}

#[test]
fn an_absolute_path_outside_the_root_is_refused() {
    let root = TempDir::new("abs");
    let outside = TempDir::new("abs-outside");
    let target = outside.write("elsewhere.c", "void main(){}\n");

    let r = PathResolver::new(Some(root.path().to_path_buf()));
    let err = r
        .resolve_input(target.to_str().unwrap())
        .expect_err("an absolute path outside the root must be refused");
    assert_eq!(code(err), ErrorCode::PathEscapesFirmwareRoot);
}

#[test]
fn a_symlink_out_of_the_root_is_refused() {
    // The case that motivates canonicalizing *before* the containment check:
    // the path is textually inside the root and physically outside it.
    let root = TempDir::new("symlink");
    let outside = TempDir::new("symlink-outside");
    let secret = outside.write("secret.c", "void main(){}\n");

    let link = root.child("looks-local.c");
    std::os::unix::fs::symlink(&secret, &link).expect("create symlink");

    let r = PathResolver::new(Some(root.path().to_path_buf()));
    let err = r
        .resolve_input("looks-local.c")
        .expect_err("a symlink out of the root must be refused");
    assert_eq!(code(err), ErrorCode::PathEscapesFirmwareRoot);

    // A symlinked *directory* is the same hole one level up.
    let linked_dir = root.child("linked-dir");
    std::os::unix::fs::symlink(outside.path(), &linked_dir).expect("create dir symlink");
    let err = r
        .resolve_input("linked-dir/secret.c")
        .expect_err("a symlinked directory must not open a way out");
    assert_eq!(code(err), ErrorCode::PathEscapesFirmwareRoot);
}

#[test]
fn an_output_may_not_exist_yet_but_must_still_land_inside() {
    let root = TempDir::new("out");
    let outside = TempDir::new("out-outside");
    let r = PathResolver::new(Some(root.path().to_path_buf()));

    let ok = r.resolve_output("firmware.hex").unwrap();
    assert_eq!(ok, root.child("firmware.hex"));
    assert!(!ok.exists(), "resolve_output must not create anything");

    let err = r
        .resolve_output(outside.child("firmware.hex").to_str().unwrap())
        .expect_err("an output outside the root must be refused");
    assert_eq!(code(err), ErrorCode::PathEscapesFirmwareRoot);

    // A parent that does not exist is a miss, not a silent mkdir.
    let err = r
        .resolve_output("no/such/dir/firmware.hex")
        .expect_err("a missing parent must be reported");
    assert_eq!(code(err), ErrorCode::PathNotFound);
}

#[test]
fn writing_through_a_pre_placed_symlink_is_refused() {
    let root = TempDir::new("outlink");
    let outside = TempDir::new("outlink-outside");
    let victim = outside.write("victim.hex", "original\n");

    let link = root.child("firmware.hex");
    std::os::unix::fs::symlink(&victim, &link).expect("create symlink");

    let r = PathResolver::new(Some(root.path().to_path_buf()));
    let err = r
        .resolve_output("firmware.hex")
        .expect_err("an existing symlink out of the root must be refused for writes");
    assert_eq!(code(err), ErrorCode::PathEscapesFirmwareRoot);
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "original\n");
}

#[test]
fn a_missing_file_and_a_directory_are_distinguished() {
    let root = TempDir::new("kinds");
    root.dir("adir");
    let r = PathResolver::new(Some(root.path().to_path_buf()));

    assert_eq!(
        code(r.resolve_input("nope.c").unwrap_err()),
        ErrorCode::PathNotFound
    );
    // A directory resolves, but is not a source file.
    assert!(r.resolve_input("adir").is_ok());
    assert_eq!(
        code(r.resolve_input_file("adir").unwrap_err()),
        ErrorCode::NotAFile
    );
}

#[test]
fn with_no_root_nothing_is_confined() {
    let outside = TempDir::new("unset");
    let target = outside.write("anywhere.c", "void main(){}\n");

    let r = PathResolver::new(None);
    assert_eq!(r.confinement(), "off");
    assert_eq!(r.resolve_input(target.to_str().unwrap()).unwrap(), target);
    // Even a symlink out of nowhere in particular is fine: there is no boundary.
    assert!(r
        .resolve_output(outside.child("new.hex").to_str().unwrap())
        .is_ok());
}

#[test]
fn empty_paths_are_rejected_rather_than_resolving_to_the_root() {
    let root = TempDir::new("empty");
    let r = PathResolver::new(Some(root.path().to_path_buf()));
    assert_eq!(
        code(r.resolve_input("").unwrap_err()),
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        code(r.resolve_output("   ").unwrap_err()),
        ErrorCode::InvalidArgument
    );
}
