use std::fs;
use tempfile::tempdir;

use mojmodelbridge::metadata::infer_mod_metadata;
use mojmodelbridge::scan::scan_project;

#[test]
fn reads_fabric_metadata_and_merges_a_project_scan() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("src/main/java/demo/model")).unwrap();
    fs::create_dir_all(root.join("src/main/resources")).unwrap();
    fs::write(root.join("src/main/java/demo/model/ModelTest.java"), "class ModelTest {}").unwrap();
    fs::write(root.join("src/main/resources/fabric.mod.json"), r#"{
        "id":"sample_mod","name":"Sample Mod","version":"1.2.3","authors":["A","B"]
    }"#).unwrap();

    let metadata = infer_mod_metadata(root);
    assert_eq!(metadata.modid, "sample_mod");
    assert_eq!(metadata.authors, vec!["A", "B"]);

    let project = scan_project(&[root.to_path_buf()]).unwrap();
    assert_eq!(project.roots.len(), 1);
    assert_eq!(project.metadata.modid, "sample_mod");
    assert_eq!(project.root_scans[0].model_candidates.len(), 1);
}
