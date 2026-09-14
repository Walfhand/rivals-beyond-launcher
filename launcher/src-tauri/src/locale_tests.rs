
fn bilingual_manifest() -> Manifest {
    let mut manifest = valid_manifest();
    manifest.schema_version = 2;
    manifest.locales = vec!["frFR".into(), "enUS".into()];
    manifest.files.retain(|entry| !entry.path.starts_with("Data/frFR/"));
    for locale in &manifest.locales {
        for path in locale_required_files(locale) {
            manifest.files.push(FileEntry { path, locale: Some(locale.clone()), size: 4, sha256: sha256_bytes(b"test") });
        }
    }
    manifest.file_count = manifest.files.len();
    manifest.total_size = manifest.files.iter().map(|entry| entry.size).sum();
    manifest
}

#[test]
fn signed_optional_packs_require_complete_chains_and_consistent_tags() {
    let manifest = bilingual_manifest();
    let (document, key) = sign(&manifest);
    assert!(load_signed_manifest(&document, key).is_ok());
    let mut broken = manifest.clone();
    broken.files.last_mut().unwrap().locale = None;
    assert!(broken.validate().is_err());
    let mut missing = manifest.clone();
    missing.files.pop();
    missing.file_count -= 1;
    missing.total_size -= 4;
    assert!(missing.validate().is_err());
    let mut wrong = manifest.clone();
    wrong.files[0].locale = Some("enUS".into());
    assert!(wrong.validate().is_err());
    let mut legacy = manifest;
    legacy.schema_version = 1;
    assert!(legacy.validate().is_err());
}

#[test]
fn initial_install_selects_matching_pack_first_and_adds_languages_without_duplicating_common_files() {
    let root = TestDir::new();
    let manifest = bilingual_manifest();
    let selected = select_language_files(&root.0, &manifest, "enUS", None).unwrap();
    assert_eq!(selected.files[0].locale.as_deref(), Some("enUS"));
    assert!(selected.files.iter().all(|entry| entry.locale.as_deref() != Some("frFR")));
    assert_eq!(selected.total_size, selected.files.iter().map(|entry| entry.size).sum::<u64>());
    for entry in &selected.files {
        let path = target_path(&root.0, &entry.path, true).unwrap();
        fs::write(path, b"test").unwrap();
    }
    let added = select_language_files(&root.0, &manifest, "enUS", Some("frFR")).unwrap();
    assert_eq!(added.files.len(), manifest.files.len());
    let retained = select_language_files(&root.0, &manifest, "frFR", None).unwrap();
    assert_eq!(retained.files.len(), manifest.files.len());
    assert_eq!(retained.files.iter().filter(|entry| entry.path == "Wow.exe").count(), 1);
    assert!(select_language_files(&root.0, &manifest, "deDE", None).is_err());
}

#[test]
fn changing_language_requires_download_and_preserves_game_preferences() {
    let root = TestDir::new();
    let manifest = bilingual_manifest();
    let loaded = LoadedManifest { manifest, sha256: "abc".into() };
    let selected = select_language_files(&root.0, &loaded.manifest, "enUS", None).unwrap();
    for entry in &selected.files {
        let path = target_path(&root.0, &entry.path, true).unwrap();
        fs::write(path, vec![b'x'; entry.size as usize]).unwrap();
    }
    write_state(&root.0, &LocalState { schema_version: 1, sequence: loaded.manifest.sequence,
        client_version: loaded.manifest.client_version.clone(), manifest_sha256: loaded.sha256.clone(), installed_locales: vec!["enUS".into()] }).unwrap();
    assert!(client_status_for_locale(&root.0, &loaded, "enUS").unwrap().can_launch);
    assert!(!client_status_for_locale(&root.0, &loaded, "frFR").unwrap().can_launch);
    let config = target_path(&root.0, "WTF/Config.wtf", true).unwrap();
    fs::write(&config, b"SET locale \"frFR\"\nSET gxResolution \"1280x720\"\nSET accountName \"private\"\n").unwrap();
    configure_client_locale(&root.0, "enUS").unwrap();
    let text = fs::read(&config).unwrap();
    assert_eq!(config_setting(&text, "locale"), Some(b"\"enUS\"".as_slice()));
    assert_eq!(config_setting(&text, "gxResolution"), Some(b"\"1280x720\"".as_slice()));
    assert_eq!(config_setting(&text, "accountName"), Some(b"\"private\"".as_slice()));
}

#[test]
fn real_downloads_install_one_pack_then_add_and_repair_the_other_in_place() {
    let root = TestDir::new();
    let mut manifest = bilingual_manifest();
    for entry in &mut manifest.files { entry.size = 4; entry.sha256 = sha256_bytes(b"test"); }
    manifest.total_size = manifest.files.len() as u64 * 4;
    let client = Client::builder().build().unwrap();
    let mut run = |locale: &str, additional: Option<&str>, repair: bool, expected: usize| {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        manifest.object_base_url = format!("http://{}/", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut requests = 0;
            while requests < expected && std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        socket.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
                        let mut request = [0; 4096];
                        socket.read(&mut request).unwrap();
                        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ntest").unwrap();
                        requests += 1;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(1)),
                    Err(error) => panic!("{error}"),
                }
            }
            assert_eq!(requests, expected);
        });
        let mut installed = Vec::new();
        let result = update_client_for_locale(&client, &root.0,
            LoadedManifest { manifest: manifest.clone(), sha256: "same-signed-payload".into() },
            "realm.example", repair, locale, additional, |progress| {
                if progress.phase == "install" { installed.push(progress.path); }
            });
        server.join().unwrap();
        let result = result.unwrap();
        assert_eq!(result.changed_files, expected);
        assert_eq!(result.available_locales, vec!["frFR", "enUS"]);
        installed
    };
    let first = run("enUS", None, false, 22);
    assert!(first[..11].iter().all(|path| path.starts_with("Data/enUS/")));
    assert!(!root.0.join("Data/frFR").exists());
    assert!(root.0.join("Data/enUS/realmlist.wtf").exists());
    let added = run("enUS", Some("frFR"), false, 11);
    assert!(added.iter().all(|path| path.starts_with("Data/frFR/")));
    run("frFR", None, true, 0);
    fs::remove_file(root.0.join("Data/enUS/locale-enUS.MPQ")).unwrap();
    let repaired = run("frFR", None, false, 1);
    assert_eq!(repaired, vec!["Data/enUS/locale-enUS.MPQ"]);
    assert_eq!(read_state(&root.0).unwrap().installed_locales.len(), 2);
}

#[test]
fn legacy_manifest_never_advertises_an_incomplete_english_pack() {
    let mut manifest = bilingual_manifest();
    manifest.schema_version = 1;
    manifest.locales.clear();
    for entry in &mut manifest.files { entry.locale = None; }
    assert_eq!(available_locales(&manifest), vec!["frFR", "enUS"]);
    manifest.files.retain(|entry| entry.path != "Data/enUS/lichking-speech-enUS.MPQ");
    assert_eq!(available_locales(&manifest), vec!["frFR"]);
    let root = TestDir::new();
    assert!(select_language_files(&root.0, &manifest, "enUS", None).is_err());
    let selected = select_language_files(&root.0, &manifest, "frFR", None).unwrap();
    assert_eq!(selected.files.len(), manifest.files.len());
}

#[test]
fn restarting_an_interrupted_additional_pack_keeps_the_requested_language() {
    let root = TestDir::new();
    let manifest = bilingual_manifest();
    let requested = select_language_files(&root.0, &manifest, "enUS", Some("frFR")).unwrap();
    write_incomplete_state(&root.0, &requested.locales).unwrap();
    assert!(!root.0.join("Data/frFR/locale-frFR.MPQ").exists());
    let resumed = select_language_files(&root.0, &manifest, "enUS", None).unwrap();
    assert_eq!(resumed.files.len(), requested.files.len());
    assert!(!client_status(&root.0).unwrap().can_launch);
}
