use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::bytecode::parse_any_model_class;
use crate::model::{expand_render_instances, ParsedModel, Part};
use crate::source::parse_java_model;

#[derive(Debug, Clone, Serialize)]
pub struct McreatorAnimationRecord {
    pub field: String,
    pub source_name: String,
    pub length: f64,
    pub channel_count: usize,
    pub helper_count: usize,
    pub walking: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelSourceExport {
    pub source: PathBuf,
    pub model_path: PathBuf,
    pub animation_path: Option<PathBuf>,
    pub class_name: String,
    pub part_count: usize,
    pub cube_count: usize,
    pub animations: Vec<McreatorAnimationRecord>,
    pub warnings: Vec<String>,
}

fn java_identifier(value: &str, fallback: &str) -> String {
    const KEYWORDS: &[&str] = &[
        "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char", "class",
        "const", "continue", "default", "do", "double", "else", "enum", "extends", "final",
        "finally", "float", "for", "goto", "if", "implements", "import", "instanceof", "int",
        "interface", "long", "native", "new", "package", "private", "protected", "public",
        "return", "short", "static", "strictfp", "super", "switch", "synchronized", "this",
        "throw", "throws", "transient", "try", "void", "volatile", "while", "record", "sealed",
    ];
    let mut result: String = value.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' { ch } else { '_' })
        .collect();
    if result.is_empty() { result = fallback.to_owned(); }
    if result.chars().next().map(|ch| ch.is_ascii_digit()).unwrap_or(false) {
        result.insert(0, '_');
    }
    if KEYWORDS.contains(&result.as_str()) { result.push('_'); }
    result
}

fn java_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

fn java_float(value: f64) -> Result<String> {
    if !value.is_finite() { bail!("Cannot emit a non-finite Java float literal."); }
    let value = if value.abs() < 5e-8 { 0.0 } else { value };
    let mut text = format!("{value:.7}");
    while text.contains('.') && text.ends_with('0') { text.pop(); }
    if text.ends_with('.') { text.push('0'); }
    if !text.contains('.') { text.push_str(".0"); }
    Ok(format!("{text}F"))
}

fn snake_case(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut output = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            let previous = chars[index - 1];
            if previous.is_ascii_lowercase() || previous.is_ascii_digit() {
                output.push('_');
            }
        }
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
        } else if !output.is_empty() && !output.ends_with('_') {
            output.push('_');
        }
    }
    let output = output.trim_matches('_').to_owned();
    if output.is_empty() { "converted_entity".to_owned() } else { output }
}

fn pascal_case(value: &str) -> String {
    let mut output = String::new();
    let mut upper = true;
    for ch in value.chars() {
        if !ch.is_ascii_alphanumeric() {
            upper = true;
            continue;
        }
        if upper {
            output.push(ch.to_ascii_uppercase());
            upper = false;
        } else {
            output.push(ch);
        }
    }
    if output.is_empty() { output = "ConvertedAnimation".to_owned(); }
    if output.chars().next().map(|ch| ch.is_ascii_digit()).unwrap_or(false) {
        output.insert_str(0, "Converted");
    }
    output
}

fn part_order(parts: &[Part]) -> Result<Vec<usize>> {
    let mut by_name = HashMap::<String, usize>::new();
    for (index, part) in parts.iter().enumerate() {
        if by_name.insert(part.name.clone(), index).is_some() {
            bail!("MCreator model export found duplicate part name '{}'.", part.name);
        }
    }

    fn visit(
        index: usize,
        parts: &[Part],
        by_name: &HashMap<String, usize>,
        state: &mut [u8],
        result: &mut Vec<usize>,
    ) -> Result<()> {
        match state[index] {
            2 => return Ok(()),
            1 => bail!("Cyclic model hierarchy involving '{}'.", parts[index].name),
            _ => {}
        }
        state[index] = 1;
        if let Some(parent) = parts[index].parent.as_ref().and_then(|name| by_name.get(name)).copied() {
            visit(parent, parts, by_name, state, result)?;
        }
        state[index] = 2;
        result.push(index);
        Ok(())
    }

    let mut sorted: Vec<usize> = (0..parts.len()).collect();
    sorted.sort_by_key(|index| (parts[*index].source_index, *index));
    let mut state = vec![0u8; parts.len()];
    let mut result = Vec::with_capacity(parts.len());
    for index in sorted {
        visit(index, parts, &by_name, &mut state, &mut result)?;
    }
    Ok(result)
}

fn part_identifiers(parts: &[Part]) -> Vec<String> {
    let mut used = HashSet::<String>::new();
    parts.iter().enumerate().map(|(index, part)| {
        let base = java_identifier(
            if part.variable.is_empty() { &part.name } else { &part.variable },
            &format!("part{index}"),
        );
        let mut candidate = base.clone();
        let mut suffix = 2usize;
        while candidate.eq_ignore_ascii_case("root") || !used.insert(candidate.to_ascii_lowercase()) {
            candidate = format!("{base}_{suffix}");
            suffix += 1;
        }
        candidate
    }).collect()
}

/// Generate the Mojmap 1.21.x EntityModel source consumed by the MCreator
/// source bridge. The generated file is a class fragment; package/import headers
/// are added by the workspace bridge when it copies the model into src/main/java.
pub fn generate_mojmap_model_java(model: &ParsedModel, class_name: &str, modid: &str) -> Result<String> {
    if model.parts.is_empty() { bail!("Cannot generate a MCreator model with no parts."); }
    let class_name = java_identifier(class_name, "ConvertedEntityModel");
    let order = part_order(&model.parts)?;
    let identifiers = part_identifiers(&model.parts);
    let by_name: HashMap<String, usize> = model.parts.iter().enumerate()
        .map(|(index, part)| (part.name.clone(), index)).collect();

    let mut lines = vec![
        "// Generated by MojModelBridge native Rust from reconstructed model geometry".to_owned(),
        "// MCreator import format: Mojmap 1.21.x / Modded Entity".to_owned(),
        "import com.mojang.math.Axis;".to_owned(),
        String::new(),
        format!("public class {class_name}<T extends Entity> extends EntityModel<T> {{"),
        "\tprivate float bridgeAgeInTicks;".to_owned(),
        "\tpublic static final ModelLayerLocation LAYER_LOCATION = new ModelLayerLocation(".to_owned(),
        format!(
            "\t\t\tResourceLocation.fromNamespaceAndPath({}, {}), \"main\");",
            java_string(modid), java_string(&snake_case(&class_name))
        ),
    ];

    for index in &order {
        lines.push(format!("\tprivate final ModelPart {};", identifiers[*index]));
    }
    lines.extend([
        String::new(),
        format!("\tpublic {class_name}(ModelPart root) {{"),
    ]);

    for index in &order {
        let part = &model.parts[*index];
        let parent = part.parent.as_ref().and_then(|name| by_name.get(name)).copied();
        let parent_expression = parent.map(|parent| format!("this.{}", identifiers[parent]))
            .unwrap_or_else(|| "root".to_owned());
        lines.push(format!(
            "\t\tthis.{} = {}.getChild({});",
            identifiers[*index], parent_expression, java_string(&part.name)
        ));
    }
    lines.extend([
        "\t}".to_owned(),
        String::new(),
        "\tpublic static LayerDefinition createBodyLayer() {".to_owned(),
        "\t\tMeshDefinition meshdefinition = new MeshDefinition();".to_owned(),
        "\t\tPartDefinition partdefinition = meshdefinition.getRoot();".to_owned(),
        String::new(),
    ]);

    for index in &order {
        let part = &model.parts[*index];
        let parent = part.parent.as_ref().and_then(|name| by_name.get(name)).copied();
        let parent_identifier = parent.map(|parent| identifiers[parent].as_str()).unwrap_or("partdefinition");
        let mut builder = "CubeListBuilder.create()".to_owned();
        for cube in &part.cubes {
            builder.push_str(&format!(".texOffs({}, {})", cube.uv_x, cube.uv_y));
            if cube.mirror { builder.push_str(".mirror()"); }
            builder.push_str(&format!(
                ".addBox({}, {}, {}, {}, {}, {}, new CubeDeformation({}))",
                java_float(cube.x)?, java_float(cube.y)?, java_float(cube.z)?,
                java_float(cube.dx)?, java_float(cube.dy)?, java_float(cube.dz)?,
                java_float(cube.inflate)?
            ));
            if cube.mirror { builder.push_str(".mirror(false)"); }
        }
        let [ox, oy, oz] = part.offset;
        let [rx, ry, rz] = part.rotation;
        let pose = if [rx, ry, rz].iter().all(|value| value.abs() < 1e-8) {
            format!("PartPose.offset({}, {}, {})", java_float(ox)?, java_float(oy)?, java_float(oz)?)
        } else {
            format!(
                "PartPose.offsetAndRotation({}, {}, {}, {}, {}, {})",
                java_float(ox)?, java_float(oy)?, java_float(oz)?,
                java_float(rx)?, java_float(ry)?, java_float(rz)?
            )
        };
        lines.push(format!(
            "\t\tPartDefinition {} = {}.addOrReplaceChild({},\n\t\t\t\t{}, {});\n",
            identifiers[*index], parent_identifier, java_string(&part.name), builder, pose
        ));
    }

    lines.extend([
        format!("\t\treturn LayerDefinition.create(meshdefinition, {}, {});",
            model.texture_width, model.texture_height),
        "\t}".to_owned(),
        String::new(),
        "\t@Override".to_owned(),
        "\tpublic void setupAnim(T entity, float limbSwing, float limbSwingAmount, float ageInTicks, float netHeadYaw, float headPitch) {".to_owned(),
        "\t\tthis.bridgeAgeInTicks = ageInTicks;".to_owned(),
        "\t\t// AnimationDefinition clips are linked by the generated MCreator living-entity element.".to_owned(),
        "\t}".to_owned(),
        String::new(),
        "\t@Override".to_owned(),
        "\tpublic void renderToBuffer(PoseStack poseStack, VertexConsumer vertexConsumer, int packedLight, int packedOverlay,".to_owned(),
        "\t\t\tfloat red, float green, float blue, float alpha) {".to_owned(),
    ]);

    let roots: Vec<usize> = order.iter().copied().filter(|index| {
        model.parts[*index].parent.as_ref().and_then(|parent| by_name.get(parent)).is_none()
    }).collect();
    let mut handled = BTreeSet::<usize>::new();
    for runtime in &model.render_runtime_rotations {
        let Some(axis) = runtime.get("axis").and_then(Value::as_str) else { continue; };
        let enum_axis = match axis { "X" => "XP", "Y" => "YP", "Z" => "ZP", _ => continue };
        let speed = runtime.get("speed").and_then(Value::as_f64).unwrap_or(0.0);
        let degrees = runtime.get("degrees").and_then(Value::as_bool).unwrap_or(false);
        let speed = if degrees { speed } else { speed.to_degrees() };
        let angle = java_float(speed)?;
        let sources: BTreeSet<String> = runtime.get("parts").and_then(Value::as_array)
            .into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
        let group: Vec<usize> = roots.iter().copied().filter(|index| {
            let part = &model.parts[*index];
            let source = if part.source_name.is_empty() { &part.name } else { &part.source_name };
            sources.contains(source)
        }).collect();
        if group.is_empty() { continue; }
        lines.push("\t\tposeStack.pushPose();".to_owned());
        lines.push(format!("\t\tposeStack.mulPose(Axis.{enum_axis}.rotationDegrees(this.bridgeAgeInTicks * {angle}));"));
        for index in group {
            lines.push(format!(
                "\t\t{}.render(poseStack, vertexConsumer, packedLight, packedOverlay, red, green, blue, alpha);",
                identifiers[index]
            ));
            handled.insert(index);
        }
        lines.push("\t\tposeStack.popPose();".to_owned());
    }
    for index in roots {
        if handled.contains(&index) { continue; }
        lines.push(format!(
            "\t\t{}.render(poseStack, vertexConsumer, packedLight, packedOverlay, red, green, blue, alpha);",
            identifiers[index]
        ));
    }
    lines.extend(["\t}".to_owned(), "}".to_owned(), String::new()]);
    Ok(lines.join("\n"))
}

fn animation_field_name(class_name: &str, animation_name: &str, used: &mut HashSet<String>) -> String {
    let suffix = animation_name.rsplit('.').next().unwrap_or(animation_name);
    let base = java_identifier(&pascal_case(&format!("{class_name} {suffix}")), "ConvertedAnimation");
    let mut value = base.clone();
    let mut suffix_number = 2usize;
    while !used.insert(value.to_ascii_lowercase()) {
        value = format!("{base}{suffix_number}");
        suffix_number += 1;
    }
    value
}

fn number_value(value: Option<&Value>) -> Option<f64> {
    value.and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse::<f64>().ok()))
}

fn animation_keyframes(animator: &Value, channel: &str) -> Vec<(f64, [f64; 3])> {
    let mut frames = Vec::new();
    let Some(keyframes) = animator.get("keyframes").and_then(Value::as_array) else { return frames; };
    for keyframe in keyframes {
        if keyframe.get("channel").and_then(Value::as_str) != Some(channel) { continue; }
        let Some(time) = number_value(keyframe.get("time")) else { continue; };
        let Some(point) = keyframe.get("data_points").and_then(Value::as_array).and_then(|points| points.first()) else { continue; };
        let Some(x) = number_value(point.get("x")) else { continue; };
        let Some(y) = number_value(point.get("y")) else { continue; };
        let Some(z) = number_value(point.get("z")) else { continue; };
        frames.push((time, [x, y, z]));
    }
    frames.sort_by(|left, right| left.0.total_cmp(&right.0));
    frames
}

#[derive(Debug, Clone, Serialize)]
pub struct McreatorAnimationRecord {
    pub field: String,
    pub source_name: String,
    pub length: f64,
    pub channel_count: usize,
    pub helper_count: usize,
    pub walking: bool,
}

#[derive(Debug, Clone)]
pub struct GeneratedAnimationJava {
    pub source: String,
    pub records: Vec<McreatorAnimationRecord>,
}

pub fn generate_animation_definition_java(model: &ParsedModel, class_name: &str) -> Result<Option<GeneratedAnimationJava>> {
    if model.blockbench_animations.is_empty() { return Ok(None); }
    let class_name = java_identifier(class_name, "ConvertedEntityModel");
    let animation_class = format!("{class_name}Animation");
    let mut used_fields = HashSet::<String>::new();
    let mut declarations = Vec::<String>::new();
    let mut methods = Vec::<String>::new();
    let mut records = Vec::<McreatorAnimationRecord>::new();
    let channel_limit = 6usize;

    for clip in &model.blockbench_animations {
        let Some(animation_name) = clip.get("name").and_then(Value::as_str) else { continue; };
        let mut additions = Vec::<String>::new();
        let mut max_time = number_value(clip.get("length")).unwrap_or(0.0);
        let animators = clip.get("animators").and_then(Value::as_object);
        let Some(animators) = animators else { continue; };

        for animator in animators.values() {
            let Some(bone_name) = animator.get("name").and_then(Value::as_str) else { continue; };
            let targets = model.parts.iter().filter(|part| {
                let source = if part.source_name.is_empty() { &part.name } else { &part.source_name };
                part.name == bone_name || source == bone_name
            }).map(|part| part.name.clone()).collect::<Vec<_>>();
            let target_bones = if targets.is_empty() { vec![bone_name.to_owned()] } else { targets };

            for (channel, target, vector_method) in [
                ("rotation", "ROTATION", "degreeVec"),
                ("position", "POSITION", "posVec"),
            ] {
                let frames = animation_keyframes(animator, channel);
                if frames.is_empty() { continue; }
                let mut frame_sources = Vec::new();
                for (time, vector) in &frames {
                    max_time = max_time.max(*time);
                    frame_sources.push(format!(
                        "new Keyframe({}, KeyframeAnimations.{}({}, {}, {}), AnimationChannel.Interpolations.LINEAR)",
                        java_float(*time)?, vector_method,
                        java_float(vector[0])?, java_float(vector[1])?, java_float(vector[2])?
                    ));
                }
                let frame_list = frame_sources.join(",\n\t\t\t\t\t\t\t");
                for target_bone in &target_bones {
                    additions.push(format!(
                        "\t\t\t.addAnimation({},\n\t\t\t\t\tnew AnimationChannel(AnimationChannel.Targets.{},\n\t\t\t\t\t\t\t{}))",
                        java_string(target_bone), target, frame_list
                    ));
                }
            }
        }

        if additions.is_empty() { continue; }
        let field_name = animation_field_name(&class_name, animation_name, &mut used_fields);
        let length = max_time.max(0.05);
        let looped = match clip.get("loop") {
            Some(Value::String(value)) => value.eq_ignore_ascii_case("loop"),
            Some(Value::Bool(value)) => *value,
            _ => true,
        };
        declarations.push(format!("\tpublic static final AnimationDefinition {field_name} = build{field_name}();"));
        let chunks: Vec<Vec<String>> = additions.chunks(channel_limit).map(|chunk| chunk.to_vec()).collect();
        let mut build = vec![
            format!("\tprivate static AnimationDefinition build{field_name}() {{"),
            format!("\t\tAnimationDefinition.Builder builder = AnimationDefinition.Builder.withLength({}){};",
                java_float(length)?, if looped { ".looping()" } else { "" }),
        ];
        for index in 0..chunks.len() {
            build.push(format!("\t\tbuilder = add{field_name}Part{}(builder);", index + 1));
        }
        build.extend(["\t\treturn builder.build();".to_owned(), "\t}".to_owned()]);
        methods.push(build.join("\n"));
        for (index, chunk) in chunks.iter().enumerate() {
            methods.push(format!(
                "\tprivate static AnimationDefinition.Builder add{field_name}Part{}(AnimationDefinition.Builder builder) {{\n\t\treturn builder\n{}\n\t}}",
                index + 1, chunk.join("\n")
            ));
        }
        let lower = animation_name.to_ascii_lowercase();
        records.push(McreatorAnimationRecord {
            field: field_name,
            source_name: animation_name.to_owned(),
            length,
            channel_count: additions.len(),
            helper_count: chunks.len(),
            walking: lower.contains("walk") || lower.contains("run"),
        });
    }

    if declarations.is_empty() { return Ok(None); }
    let source = [
        "// Generated by MojModelBridge native Rust from sampled animation keyframes",
        "// Large clips are split into helper methods to avoid the JVM method-size limit.",
        "",
    ].into_iter().map(str::to_owned).chain([
        format!("public class {animation_class} {{"),
        declarations.join("\n"),
        String::new(),
        methods.join("\n\n"),
        "}".to_owned(),
        String::new(),
    ]).collect::<Vec<_>>().join("\n");
    Ok(Some(GeneratedAnimationJava { source, records }))
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelSourceExport {
    pub source: PathBuf,
    pub model_path: PathBuf,
    pub animation_path: Option<PathBuf>,
    pub class_name: String,
    pub part_count: usize,
    pub cube_count: usize,
    pub animations: Vec<McreatorAnimationRecord>,
    pub warnings: Vec<String>,
}

/// Parse a Java or compiled class model, expand repeated render instances, and
/// write MCreator staging Java files under models/mojmap-1.21.x and models/animations.
pub fn export_model_sources(
    source: &Path,
    output_root: &Path,
    modid: &str,
    requested_class_name: Option<&str>,
    target_generator: &str,
    overwrite: bool,
) -> Result<ModelSourceExport> {
    if !source.is_file() {
        bail!("Model source does not exist or is not a file: {}", source.display());
    }
    let extension = source.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut model = match extension.as_str() {
        "java" => {
            let text = fs::read_to_string(source).with_context(|| format!("Reading {}", source.display()))?;
            parse_java_model(&text, &source.to_string_lossy(), &[])?
        }
        "class" => parse_any_model_class(source, &[])?,
        _ => bail!("MCreator model export accepts .java or .class input."),
    };
    expand_render_instances(&mut model);
    if model.cube_count() == 0 {
        bail!("No model cubes were recovered from {}. {}", source.display(), model.warnings.join(" | "));
    }

    let class_name = java_identifier(
        requested_class_name.unwrap_or(&model.class_name),
        "ConvertedEntityModel",
    );
    let generator_folder = if target_generator == "neoforge-1.21.1" {
        "mojmap-1.21.x"
    } else if target_generator == "forge-1.20.1" {
        "mojmap-1.20.x"
    } else {
        bail!("Unsupported MCreator model target generator '{target_generator}'.");
    };
    let models_dir = output_root.join("models").join(generator_folder);
    let animations_dir = output_root.join("models").join("animations");
    let model_path = models_dir.join(format!("{class_name}.java"));
    let generated_animation = generate_animation_definition_java(&model, &class_name)?;
    let animation_path = generated_animation.as_ref().map(|_| {
        animations_dir.join(format!("{class_name}Animation.java"))
    });
    if !overwrite {
        if model_path.exists() { bail!("Output already exists: {}", model_path.display()); }
        if let Some(path) = animation_path.as_ref().filter(|path| path.exists()) {
            bail!("Output already exists: {}", path.display());
        }
    }

    let model_java = generate_mojmap_model_java(&model, &class_name, modid)?;
    fs::create_dir_all(&models_dir)?;
    fs::write(&model_path, format!("{model_java}\n"))
        .with_context(|| format!("Writing MCreator model {}", model_path.display()))?;
    let mut animation_records = Vec::new();
    if let (Some(path), Some(animation)) = (animation_path.as_ref(), generated_animation.as_ref()) {
        fs::create_dir_all(&animations_dir)?;
        fs::write(path, format!("{}\n", animation.source))
            .with_context(|| format!("Writing MCreator animation {}", path.display()))?;
        animation_records = animation.records.clone();
    }

    Ok(ModelSourceExport {
        source: source.to_path_buf(),
        model_path,
        animation_path,
        class_name,
        part_count: model.parts.len(),
        cube_count: model.cube_count(),
        animations: animation_records,
        warnings: model.warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cube, Part};
    use serde_json::json;
    use tempfile::tempdir;

    fn model() -> ParsedModel {
        let mut model = ParsedModel::new("ModelWing".into(), 64, 64, vec![
            Part {
                name: "body".into(), variable: "body".into(), parent_variable: "root".into(),
                offset: [0.0, 0.0, 0.0], rotation: [0.0; 3],
                cubes: vec![Cube { x:-4.0,y:0.0,z:-2.0,dx:8.0,dy:12.0,dz:4.0,uv_x:0,uv_y:0,mirror:false,inflate:0.25,name:"body_cube".into() }],
                parent: None, children: vec!["wing".into()], source_index:0, source_name:"body".into(),
                render_instance_index:0,render_instance_spec:Value::Null,origin:[0.0,24.0,0.0],group_uuid:String::new()
            },
            Part {
                name: "wing".into(), variable: "wing".into(), parent_variable: "body".into(),
                offset: [0.0, -2.0, 0.0], rotation: [0.0, 0.0, 0.2],
                cubes: vec![Cube { x:-1.0,y:-1.0,z:-1.0,dx:2.0,dy:2.0,dz:2.0,uv_x:32,uv_y:0,mirror:true,inflate:0.0,name:"wing_cube".into() }],
                parent: Some("body".into()),children:Vec::new(),source_index:1,source_name:"wing".into(),
                render_instance_index:0,render_instance_spec:Value::Null,origin:[0.0,24.0,0.0],group_uuid:String::new()
            }
        ], Vec::new());
        model.render_runtime_rotations.push(json!({"axis":"Y","degrees":true,"speed":2.0,"parts":["body"]}));
        model.blockbench_animations = vec![json!({
            "name":"animation.model.walk","loop":"loop","length":1.0,
            "animators":{"bone1":{"name":"wing","type":"bone","keyframes":[
                {"channel":"rotation","time":0.0,"data_points":[{"x":0.0,"y":0.0,"z":0.0}]},
                {"channel":"rotation","time":1.0,"data_points":[{"x":1.0,"y":0.0,"z":0.0}]},
                {"channel":"position","time":0.0,"data_points":[{"x":0.0,"y":2.0,"z":0.0}]}
            ]}}
        })];
        model
    }

    #[test]
    fn emits_mojmap_geometry_hierarchy_and_render_runtime() {
        let source = generate_mojmap_model_java(&model(), "ModelWing", "demo_mod").unwrap();
        assert!(source.contains("extends EntityModel<T>"));
        assert!(source.contains("ResourceLocation.fromNamespaceAndPath(\"demo_mod\", \"model_wing\")"));
        assert!(source.contains("addOrReplaceChild(\"wing\""));
        assert!(source.contains("new CubeDeformation(0.25F)"));
        assert!(source.contains(".mirror()"));
        assert!(source.contains("PartPose.offsetAndRotation"));
        assert!(source.contains("Axis.YP.rotationDegrees(this.bridgeAgeInTicks * 2.0F)"));
        assert!(source.contains("this.body.render(poseStack"));
    }

    #[test]
    fn converts_blockbench_clips_to_split_animation_definitions_and_cloned_bones() {
        let mut model = model();
        let mut clone = model.parts[1].clone();
        clone.name = "wing_2".into();
        clone.variable = "wing_2".into();
        clone.render_instance_index = 1;
        model.parts.push(clone);

        let generated = generate_animation_definition_java(&model, "ModelWing").unwrap().unwrap();
        assert!(generated.source.contains("ModelWingWalk"));
        assert!(generated.source.contains("AnimationDefinition.Builder.withLength(1.0F).looping()"));
        assert!(generated.source.contains(".addAnimation(\"wing\""));
        assert!(generated.source.contains(".addAnimation(\"wing_2\""));
        assert!(generated.source.contains("KeyframeAnimations.degreeVec"));
        assert!(generated.source.contains("KeyframeAnimations.posVec"));
        assert_eq!(generated.records.len(), 1);
        assert_eq!(generated.records[0].channel_count, 4);
    }

    #[test]
    fn exports_workspace_staging_paths_from_java_source() {
        let root = tempdir().unwrap();
        let source = root.path().join("ModelExport.java");
        fs::write(&source, r#"
            public class ModelExport {
                public static LayerDefinition createBodyLayer() {
                    MeshDefinition mesh = new MeshDefinition();
                    PartDefinition root = mesh.getRoot();
                    root.addOrReplaceChild("body",
                        CubeListBuilder.create().texOffs(0, 0).addBox(-1, -1, -1, 2, 2, 2),
                        PartPose.ZERO);
                    return LayerDefinition.create(mesh, 64, 64);
                }
            }
        "#).unwrap();
        let out = root.path().join("workspace");
        let result = export_model_sources(&source, &out, "demo", None, "neoforge-1.21.1", false).unwrap();
        assert!(result.model_path.ends_with("models/mojmap-1.21.x/ModelExport.java"));
        assert!(result.model_path.is_file());
        assert_eq!(result.part_count, 1);
        assert_eq!(result.cube_count, 1);
    }
}
