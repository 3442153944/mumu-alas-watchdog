//! 配置草稿与保存回归测试。

use super::*;
#[test]
fn merge_keeps_custom_fields_and_adds_missing_options() {
    let mut draft: Value =
        serde_yaml::from_str("schedule:\n  restart_interval_hours: 12\ncustom: keep\n").unwrap();
    merge_defaults(
        &mut draft,
        &serde_yaml::from_str::<Value>(DEFAULT_CONFIG_TEXT).unwrap(),
    );
    assert_eq!(
        draft["schedule"]["restart_interval_hours"].as_i64(),
        Some(12)
    );
    assert_eq!(draft["custom"].as_str(), Some("keep"));
    assert_eq!(draft["health"]["adb_fail_threshold"].as_i64(), Some(3));
    assert!(validate(&draft).is_ok());
}
#[test]
fn missing_program_path_is_rejected() {
    let mut draft: Value = serde_yaml::from_str(DEFAULT_CONFIG_TEXT).unwrap();
    draft["paths"]["adb_exe"] = Value::String("Z:/__missing_watchdog__/adb.exe".into());
    assert!(validate(&draft).is_err());
}
#[test]
fn saving_preserves_unknown_keys_and_detects_external_changes() {
    let path = std::env::temp_dir().join(format!("watchdog-editor-{}.yml", std::process::id()));
    fs::write(&path, format!("{DEFAULT_CONFIG_TEXT}\ncustom: keep\n")).unwrap();
    let mut editor = ConfigEditor::load(&path);
    editor.draft["schedule"]["restart_interval_hours"] = Value::Number(12.into());
    let settings = editor.save().unwrap();
    assert_eq!(settings.restart_interval_hours, 12.0);
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("custom: keep"));
    fs::write(&path, format!("{saved}\n# external edit\n")).unwrap();
    assert!(editor.save().is_err());
    fs::remove_file(path.with_extension("yml.bak")).unwrap();
    fs::remove_file(path).unwrap();
}
