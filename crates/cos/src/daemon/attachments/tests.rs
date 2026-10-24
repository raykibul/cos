use std::process::Command;

use super::*;

#[test]
fn names_are_made_safe() {
    assert_eq!(file_name("photo.png", None), "photo.png");
    assert_eq!(
        file_name("/Users/me/Desktop/Screen Shot 1.png", None),
        "Screen-Shot-1.png"
    );
    assert_eq!(file_name("C:\\docs\\spec v2.pdf", None), "spec-v2.pdf");
    assert_eq!(file_name("../../etc/passwd", None), "passwd");
    assert_eq!(file_name(".gitignore", None), "gitignore");
    assert_eq!(file_name("..", None), "file");
    assert_eq!(file_name("", Some("image/png")), "file.png");
    assert_eq!(file_name("a&b*c?.txt", None), "a-b-c-.txt");
    assert_eq!(file_name("ছবি.jpg", None), "ছবি.jpg");
    assert_eq!(file_name("image", Some("image/jpeg; q=1")), "image.jpg");
    assert_eq!(file_name("notes", Some("text/plain")), "notes.txt");
    assert_eq!(file_name("Makefile", Some("application/x-unknown")), "Makefile");
    let long = format!("{}.png", "x".repeat(300));
    let name = file_name(&long, None);
    assert_eq!(name.chars().count(), MAX_NAME_CHARS);
    assert!(name.ends_with("x.png"));
}

#[test]
fn content_types_come_from_the_extension() {
    assert_eq!(content_type_of("a.PNG"), "image/png");
    assert_eq!(content_type_of("a.jpeg"), "image/jpeg");
    assert_eq!(content_type_of("spec.pdf"), "application/pdf");
    assert_eq!(content_type_of("Makefile"), "application/octet-stream");
}

#[test]
fn sizes_read_well() {
    assert_eq!(size_label(812), "812 B");
    assert_eq!(size_label(250_000), "245 KB");
    assert_eq!(size_label(3_500_000), "3.3 MB");
}

#[test]
fn saving_keeps_names_unique_and_out_of_git() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let git = |args: &[&str]| {
        let out = Command::new("git").args(args).current_dir(repo).output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    git(&["init", "-q"]);

    let first = save(repo, "photo.png", Some("image/png"), b"one").unwrap();
    let second = save(repo, "photo.png", Some("image/png"), b"two").unwrap();
    assert_eq!(first.name, "photo.png");
    assert_eq!(second.name, "photo-2.png");
    assert_eq!(first.path, repo.join(".cos/attachments/photo.png"));
    assert_eq!(first.content_type, "image/png");
    assert_eq!(second.size, 3);
    assert_eq!(std::fs::read(&second.path).unwrap(), b"two");
    assert_eq!(git(&["status", "--porcelain"]), "", "attachments stay out of git");

    assert_eq!(find(repo, "photo-2.png").unwrap(), second);
    assert_eq!(find(repo, second.path.to_str().unwrap()).unwrap(), second);
    for missing in [
        "nope.png",
        ".gitignore",
        "../attachments/photo.png",
        "/etc/passwd",
        "",
    ] {
        assert_eq!(
            find(repo, missing).unwrap_err().status,
            StatusCode::NOT_FOUND,
            "{missing}"
        );
    }

    let names = vec![
        "photo.png".to_owned(),
        "photo.png".to_owned(),
        "photo-2.png".to_owned(),
    ];
    assert_eq!(find_all(repo, &names).unwrap(), vec![first.clone(), second]);
    let many = vec!["photo.png".to_owned(); MAX_PER_MESSAGE + 1];
    assert_eq!(find_all(repo, &many).unwrap_err().status, StatusCode::BAD_REQUEST);

    let other = tempfile::tempdir().unwrap();
    let copied = copy_into(&first, other.path()).unwrap();
    assert_eq!(copied.path, other.path().join(".cos/attachments/photo.png"));
    assert_eq!(std::fs::read(&copied.path).unwrap(), b"one");
    assert_eq!(copy_into(&first, repo).unwrap(), first, "already there");
    assert_eq!(
        copy_into(&first, other.path()).unwrap(),
        copied,
        "handed over before"
    );
}

#[test]
fn large_files_and_symlinked_folders_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let big = vec![0u8; MAX_BYTES + 1];
    assert_eq!(
        save(dir.path(), "big.bin", None, &big).unwrap_err().status,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let elsewhere = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), dir.path().join(".cos")).unwrap();
    assert!(save(dir.path(), "a.txt", None, b"x").is_err());
    assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
}

#[test]
fn the_block_round_trips() {
    let attachment = |name: &str, size| Attachment {
        name: name.into(),
        path: PathBuf::from(format!("/w/My Repo/.cos/attachments/{name}")),
        size,
        content_type: content_type_of(name).into(),
    };
    let files = [
        attachment("photo.png", 250_000),
        attachment("spec.pdf", 3_500_000),
    ];
    let text = with_attachments("Fix the layout in this screenshot.\n", &files);
    assert_eq!(
        text,
        "Fix the layout in this screenshot.\n\n\
         Attached files (saved by cos; read them with the Read tool):\n\
         - /w/My Repo/.cos/attachments/photo.png (image/png, 245 KB)\n\
         - /w/My Repo/.cos/attachments/spec.pdf (application/pdf, 3.3 MB)"
    );
    let (rest, attached) = split(&text);
    assert_eq!(rest, "Fix the layout in this screenshot.");
    assert_eq!(attached.len(), 2);
    assert_eq!(attached[0].name, "photo.png");
    assert_eq!(attached[0].path, "/w/My Repo/.cos/attachments/photo.png");
    assert_eq!(attached[0].content_type, "image/png");
    assert_eq!(attached[1].size_label, "3.3 MB");

    let only = with_attachments("", &files[..1]);
    let (rest, attached) = split(&only);
    assert_eq!(rest, "");
    assert_eq!(attached.len(), 1);

    assert_eq!(with_attachments("hi", &[]), "hi");
    assert_eq!(split("plain text"), ("plain text".to_owned(), Vec::new()));
    let quoted = format!("{HEADER}\nsomething else");
    assert_eq!(split(&quoted), (quoted.clone(), Vec::new()));
}
