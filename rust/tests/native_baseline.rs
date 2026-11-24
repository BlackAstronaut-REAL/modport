use std::fs;
use tempfile::tempdir;

use mojmodelbridge::metadata::infer_mod_metadata;
use mojmodelbridge::pipeline::convert_model_source;
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

#[test]
fn converts_modern_mojmap_java_model_to_blockbench() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("ModelModern.java");
    let output = temp.path().join("out/ModelModern.bbmodel");
    fs::write(&source, r#"
        public class ModelModern {
            private static final float INFLATE = 0.25F;
            public static LayerDefinition createBodyLayer() {
                MeshDefinition mesh = new MeshDefinition();
                PartDefinition root = mesh.getRoot();
                PartDefinition body = root.addOrReplaceChild("body",
                    CubeListBuilder.create().texOffs(0, 0)
                        .addBox(-4, 0, -2, 8, 12, 4, new CubeDeformation(INFLATE)),
                    PartPose.offset(0, 0, 0));
                body.addOrReplaceChild("head",
                    CubeListBuilder.create().texOffs(32, 0).addBox(-4, -8, -4, 8, 8, 8),
                    PartPose.offset(0, -6, 0));
                return LayerDefinition.create(mesh, 64, 64);
            }
            public void setupAnim(Entity entity, float a, float b, float c, float d, float e) {
                this.head.xRot = e;
            }
        }
    "#).unwrap();

    let converted = convert_model_source(&source, &output, &[], false).unwrap();
    assert_eq!(converted.source_format, "modern_mojmap_java");
    assert_eq!(converted.part_count, 2);
    assert_eq!(converted.cube_count, 2);

    let project: serde_json::Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(project["resolution"]["width"], 64);
    assert_eq!(project["resolution"]["height"], 64);
    assert_eq!(project["elements"].as_array().unwrap().len(), 2);
    assert_eq!(project["outliner"][0]["children"].as_array().unwrap().len(), 2);
    assert!(temp.path().join("out/ModelModern_animation.java").is_file());
}

#[test]
fn converts_legacy_modelrenderer_java_model_to_blockbench() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("ModelLegacy.java");
    let output = temp.path().join("legacy/ModelLegacy.bbmodel");
    fs::write(&source, r#"
        public class ModelLegacy extends ModelBase {
            public ModelRenderer body;
            public ModelRenderer head;
            public ModelLegacy() {
                this.textureWidth = 64;
                this.textureHeight = 32;
                this.body = new ModelRenderer(this, 0, 0);
                this.body.setRotationPoint(0, 4, 0);
                this.body.addBox(-2, -3, -1, 4, 6, 2, 0.25F);
                this.head = new ModelRenderer(this, 16, 0);
                this.head.addBox(-1, -1, -1, 2, 2, 2);
                this.body.addChild(this.head);
            }
        }
    "#).unwrap();

    let converted = convert_model_source(&source, &output, &[], false).unwrap();
    assert_eq!(converted.source_format, "legacy_java_source");
    assert_eq!(converted.part_count, 2);
    assert_eq!(converted.cube_count, 2);
    let project: serde_json::Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(project["elements"].as_array().unwrap().len(), 2);
    assert_eq!(project["resolution"]["height"], 32);
}
