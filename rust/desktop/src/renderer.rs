use eframe::egui::{self, Color32, ColorImage};
use image::RgbaImage;
use mojmodelbridge::viewer::{NodePose, ViewerCube, ViewerNode, ViewerScene};

#[derive(Debug, Clone)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    target: [f32; 3],
    initialized: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: 0.62,
            pitch: 0.24,
            distance: 0.0,
            target: [0.0; 3],
            initialized: false,
        }
    }
}

impl Camera {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn orbit(&mut self, delta_x: f32, delta_y: f32) {
        self.yaw += delta_x * 0.009;
        self.pitch = (self.pitch + delta_y * 0.009).clamp(-1.48, 1.48);
    }

    pub fn zoom(&mut self, wheel_delta: f32) {
        if self.distance > 0.0 {
            self.distance = (self.distance * (1.0 - wheel_delta * 0.0015)).clamp(8.0, 1200.0);
        }
    }

    pub fn pan(&mut self, delta_x: f32, delta_y: f32) {
        let scale = self.distance.max(24.0) * 0.0025;
        self.target[0] -= delta_x * scale;
        self.target[1] += delta_y * scale;
    }
}

#[derive(Clone, Copy)]
struct Transform {
    rotation: [[f32; 3]; 3],
    translation: [f32; 3],
}

impl Transform {
    fn identity() -> Self {
        Self {
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            translation: [0.0; 3],
        }
    }

    fn from_pose(pose: NodePose) -> Self {
        let (sx, cx) = pose.rotation[0].sin_cos();
        let (sy, cy) = pose.rotation[1].sin_cos();
        let (sz, cz) = pose.rotation[2].sin_cos();
        let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
        let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
        let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
        Self {
            rotation: multiply(multiply(rz, ry), rx),
            translation: pose.translation,
        }
    }

    fn point(self, value: [f32; 3]) -> [f32; 3] {
        add(multiply_vector(self.rotation, value), self.translation)
    }

    fn direction(self, value: [f32; 3]) -> [f32; 3] {
        multiply_vector(self.rotation, value)
    }

    fn combine(self, child: Self) -> Self {
        Self {
            rotation: multiply(self.rotation, child.rotation),
            translation: self.point(child.translation),
        }
    }
}

#[derive(Clone, Copy)]
struct MeshVertex {
    position: [f32; 3],
    uv: [f32; 2],
}

#[derive(Clone, Copy)]
struct ProjectedVertex {
    x: f32,
    y: f32,
    z: f32,
    u: f32,
    v: f32,
}

pub fn render_scene(
    scene: &ViewerScene,
    pose: &[NodePose],
    texture: Option<&RgbaImage>,
    dimensions: [usize; 2],
    camera: &mut Camera,
) -> ColorImage {
    let textures = texture.into_iter().cloned().collect::<Vec<_>>();
    render_scene_with_textures(scene, pose, &textures, 0, dimensions, camera)
}

pub fn render_scene_with_textures(
    scene: &ViewerScene,
    pose: &[NodePose],
    textures: &[RgbaImage],
    default_texture_index: usize,
    dimensions: [usize; 2],
    camera: &mut Camera,
) -> ColorImage {
    let width = dimensions[0].clamp(96, 1024);
    let height = dimensions[1].clamp(96, 1024);
    let background = [22_u8, 27_u8, 38_u8, 255_u8];
    let mut pixels = vec![0_u8; width * height * 4];
    for pixel in pixels.chunks_exact_mut(4) { pixel.copy_from_slice(&background); }
    let mut depth = vec![f32::INFINITY; width * height];

    let poses = if pose.len() == scene.nodes.len() {
        pose.to_vec()
    } else {
        scene.nodes.iter().map(|node| NodePose {
            translation: node.local_translation,
            rotation: node.base_rotation,
        }).collect()
    };
    let worlds = world_transforms(&scene.nodes, &poses);
    let (target, radius) = scene_bounds(&scene.nodes, &worlds);
    if !camera.initialized {
        camera.target = target;
        camera.distance = (radius * 3.6).max(36.0);
        camera.initialized = true;
    }

    let aspect = width as f32 / height as f32;
    let focal = (width.min(height) as f32 * 1.45).max(100.0);
    let light = normalize([-0.36, -0.72, 0.58]);

    for (index, node) in scene.nodes.iter().enumerate() {
        let Some(world) = worlds.get(index).copied() else { continue; };
        for cube in &node.cubes {
            draw_cube(
                cube, world, camera, textures, default_texture_index, scene.texture_size,
                width, height, focal, aspect, light, &mut pixels, &mut depth,
            );
        }
    }

    ColorImage::from_rgba_unmultiplied([width, height], &pixels)
}

fn world_transforms(nodes: &[ViewerNode], poses: &[NodePose]) -> Vec<Transform> {
    fn visit(
        index: usize,
        nodes: &[ViewerNode],
        poses: &[NodePose],
        cache: &mut [Option<Transform>],
        visiting: &mut [bool],
    ) -> Transform {
        if let Some(value) = cache[index] { return value; }
        if visiting[index] { return Transform::identity(); }
        visiting[index] = true;
        let local = Transform::from_pose(poses[index]);
        let world = nodes[index].parent
            .filter(|parent| *parent < nodes.len() && *parent != index)
            .map(|parent| visit(parent, nodes, poses, cache, visiting).combine(local))
            .unwrap_or(local);
        visiting[index] = false;
        cache[index] = Some(world);
        world
    }

    let mut cache = vec![None; nodes.len()];
    let mut visiting = vec![false; nodes.len()];
    for index in 0..nodes.len() {
        let _ = visit(index, nodes, poses, &mut cache, &mut visiting);
    }
    cache.into_iter().map(|value| value.unwrap_or_else(Transform::identity)).collect()
}

fn scene_bounds(nodes: &[ViewerNode], worlds: &[Transform]) -> ([f32; 3], f32) {
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for (index, node) in nodes.iter().enumerate() {
        let Some(world) = worlds.get(index).copied() else { continue; };
        for cube in &node.cubes {
            for x in [cube.min[0], cube.max[0]] {
                for y in [cube.min[1], cube.max[1]] {
                    for z in [cube.min[2], cube.max[2]] {
                        let point = world.point([x, y, z]);
                        for axis in 0..3 {
                            minimum[axis] = minimum[axis].min(point[axis]);
                            maximum[axis] = maximum[axis].max(point[axis]);
                        }
                    }
                }
            }
        }
    }
    if !minimum[0].is_finite() {
        return ([0.0, 12.0, 0.0], 16.0);
    }
    let center = std::array::from_fn(|axis| (minimum[axis] + maximum[axis]) * 0.5);
    let extent = std::array::from_fn::<_, 3, _>(|axis| maximum[axis] - minimum[axis]);
    let radius = ((extent[0] * extent[0] + extent[1] * extent[1] + extent[2] * extent[2]).sqrt() * 0.5).max(4.0);
    (center, radius)
}

fn draw_cube(
    cube: &ViewerCube,
    world: Transform,
    camera: &Camera,
    textures: &[RgbaImage],
    default_texture_index: usize,
    texture_size: [u32; 2],
    width: usize,
    height: usize,
    focal: f32,
    _aspect: f32,
    light: [f32; 3],
    pixels: &mut [u8],
    depth_buffer: &mut [f32],
) {
    let [x0, y0, z0] = cube.min;
    let [x1, y1, z1] = cube.max;
    let u = cube.uv_origin[0];
    let v = cube.uv_origin[1];
    let tw = texture_size[0].max(1) as f32;
    let th = texture_size[1].max(1) as f32;
    let dz = z1 - z0;
    let dx = x1 - x0;
    let dy = y1 - y0;

    // Store face geometry first, then choose the correct image and UV scaling
    // per face. Legacy cubes have no explicit texture map and use texture 0.
    let faces = [
        (0, [[x1,y0,z1],[x1,y0,z0],[x1,y1,z0],[x1,y1,z1]], [u+dz+dx, v+dz, u+2.0*dz+dx, v+dz+dy], [1.0,0.0,0.0]),
        (1, [[x0,y0,z0],[x0,y0,z1],[x0,y1,z1],[x0,y1,z0]], [u, v+dz, u+dz, v+dz+dy], [-1.0,0.0,0.0]),
        (2, [[x1,y0,z1],[x0,y0,z1],[x0,y0,z0],[x1,y0,z0]], [u+dz, v, u+dz+dx, v+dz], [0.0,-1.0,0.0]),
        (3, [[x1,y1,z0],[x0,y1,z0],[x0,y1,z1],[x1,y1,z1]], [u+dz+dx, v+dz, u+dz+2.0*dx, v], [0.0,1.0,0.0]),
        (4, [[x1,y0,z0],[x0,y0,z0],[x0,y1,z0],[x1,y1,z0]], [u+dz, v+dz, u+dz+dx, v+dz+dy], [0.0,0.0,-1.0]),
        (5, [[x0,y0,z1],[x1,y0,z1],[x1,y1,z1],[x0,y1,z1]], [u+2.0*dz+dx, v+dz, u+2.0*dz+2.0*dx, v+dz+dy], [0.0,0.0,1.0]),
    ];

    for (face_index, points, default_rect, normal) in faces {
        let texture_index = cube.face_texture_indices
            .and_then(|indices| indices.get(face_index).copied())
            .unwrap_or(default_texture_index);
        let texture = textures.get(texture_index).or_else(|| textures.first());
        let (texture_width, texture_height) = texture
            .map(|image| (image.width().max(1) as f32, image.height().max(1) as f32))
            .unwrap_or((texture_size[0].max(1) as f32, texture_size[1].max(1) as f32));
        let vertices = cube_face_quad(
            cube, face_index, points, default_rect,
            texture_width, texture_height,
        );

        let world_normal = normalize(world.direction(normal));
        let view_normal = rotate_camera(world_normal, camera.yaw, camera.pitch);
        let illumination = (0.25 + dot(view_normal, light).max(0.0) * 0.75).clamp(0.18, 1.0);
        let projected: Option<[ProjectedVertex; 4]> = vertices.map(|vertex| {
            project_vertex(vertex, world, camera, width, height, focal)
        }).into_iter().collect::<Option<Vec<_>>>()
            .and_then(|values| values.try_into().ok());
        let Some(projected) = projected else { continue; };
        raster_triangle(
            [projected[0], projected[1], projected[2]], texture, illumination,
            width, height, pixels, depth_buffer,
        );
        raster_triangle(
            [projected[0], projected[2], projected[3]], texture, illumination,
            width, height, pixels, depth_buffer,
        );
    }
}

fn cube_face_quad(
    cube: &ViewerCube,
    face_index: usize,
    points: [[f32; 3]; 4],
    default_rect: [f32; 4],
    texture_width: f32,
    texture_height: f32,
) -> [MeshVertex; 4] {
    let rect = cube.face_uvs
        .and_then(|faces| faces.get(face_index).copied())
        .unwrap_or(default_rect);
    quad_from_rect(points, rect, cube.mirror, texture_width, texture_height)
}

fn quad_from_rect(
    points: [[f32; 3]; 4],
    rect: [f32; 4],
    mirror: bool,
    texture_width: f32,
    texture_height: f32,
) -> [MeshVertex; 4] {
    let [u1, v1, u2, v2] = rect;
    quad(
        points,
        [[u2, v1], [u1, v1], [u1, v2], [u2, v2]],
        mirror,
        texture_width,
        texture_height,
    )
}

fn quad(
    points: [[f32; 3]; 4],
    uv: [[f32; 2]; 4],
    mirror: bool,
    texture_width: f32,
    texture_height: f32,
) -> [MeshVertex; 4] {
    let min_u = uv.iter().map(|point| point[0]).fold(f32::INFINITY, f32::min);
    let max_u = uv.iter().map(|point| point[0]).fold(f32::NEG_INFINITY, f32::max);
    std::array::from_fn(|index| {
        let mapped_u = if mirror { min_u + max_u - uv[index][0] } else { uv[index][0] };
        MeshVertex {
            position: points[index],
            uv: [mapped_u / texture_width, uv[index][1] / texture_height],
        }
    })
}

fn project_vertex(
    vertex: MeshVertex,
    world: Transform,
    camera: &Camera,
    width: usize,
    height: usize,
    focal: f32,
) -> Option<ProjectedVertex> {
    let world_point = world.point(vertex.position);
    let relative = [
        world_point[0] - camera.target[0],
        world_point[1] - camera.target[1],
        world_point[2] - camera.target[2],
    ];
    let rotated = rotate_camera(relative, camera.yaw, camera.pitch);
    let z = rotated[2] + camera.distance;
    if z <= 0.5 { return None; }
    Some(ProjectedVertex {
        x: width as f32 * 0.5 + rotated[0] * focal / z,
        y: height as f32 * 0.5 - rotated[1] * focal / z,
        z,
        u: vertex.uv[0],
        v: vertex.uv[1],
    })
}

fn rotate_camera(point: [f32; 3], yaw: f32, pitch: f32) -> [f32; 3] {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let x = point[0] * cy + point[2] * sy;
    let z = -point[0] * sy + point[2] * cy;
    [x, point[1] * cp - z * sp, point[1] * sp + z * cp]
}

fn raster_triangle(
    vertices: [ProjectedVertex; 3],
    texture: Option<&RgbaImage>,
    shade: f32,
    width: usize,
    height: usize,
    pixels: &mut [u8],
    depth_buffer: &mut [f32],
) {
    let [a, b, c] = vertices;
    let area = edge(a.x, a.y, b.x, b.y, c.x, c.y);
    if area.abs() < 0.0001 { return; }
    let min_x = a.x.min(b.x).min(c.x).floor().max(0.0) as usize;
    let max_x = a.x.max(b.x).max(c.x).ceil().min((width - 1) as f32) as usize;
    let min_y = a.y.min(b.y).min(c.y).floor().max(0.0) as usize;
    let max_y = a.y.max(b.y).max(c.y).ceil().min((height - 1) as f32) as usize;
    if min_x > max_x || min_y > max_y { return; }

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let w0 = edge(b.x, b.y, c.x, c.y, px, py) / area;
            let w1 = edge(c.x, c.y, a.x, a.y, px, py) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < -0.001 || w1 < -0.001 || w2 < -0.001 { continue; }

            let inv_z = w0 / a.z + w1 / b.z + w2 / c.z;
            if inv_z <= 0.0 { continue; }
            let depth = 1.0 / inv_z;
            let pixel_index = y * width + x;
            if depth >= depth_buffer[pixel_index] { continue; }

            let u = (w0 * a.u / a.z + w1 * b.u / b.z + w2 * c.u / c.z) / inv_z;
            let v = (w0 * a.v / a.z + w1 * b.v / b.z + w2 * c.v / c.z) / inv_z;
            let mut color = texture.map(|image| sample_texture(image, u, v))
                .unwrap_or([175, 180, 192, 255]);
            for channel in 0..3 {
                color[channel] = (color[channel] as f32 * shade).clamp(0.0, 255.0) as u8;
            }
            if color[3] == 0 { continue; }
            blend_pixel(&mut pixels[pixel_index * 4..pixel_index * 4 + 4], color);
            depth_buffer[pixel_index] = depth;
        }
    }
}

fn sample_texture(image: &RgbaImage, u: f32, v: f32) -> [u8; 4] {
    let x = (u.clamp(0.0, 1.0) * (image.width().saturating_sub(1)) as f32).round() as u32;
    let y = (v.clamp(0.0, 1.0) * (image.height().saturating_sub(1)) as f32).round() as u32;
    let pixel = image.get_pixel(x, y).0;
    [pixel[0], pixel[1], pixel[2], pixel[3]]
}

fn blend_pixel(destination: &mut [u8], source: [u8; 4]) {
    let alpha = source[3] as f32 / 255.0;
    for channel in 0..3 {
        destination[channel] = (source[channel] as f32 * alpha
            + destination[channel] as f32 * (1.0 - alpha)).round() as u8;
    }
    destination[3] = 255;
}

fn edge(ax: f32, ay: f32, bx: f32, by: f32, px: f32, py: f32) -> f32 {
    (px - ax) * (by - ay) - (py - ay) * (bx - ax)
}

fn multiply(left: [[f32; 3]; 3], right: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut result = [[0.0; 3]; 3];
    for row in 0..3 {
        for column in 0..3 {
            for index in 0..3 { result[row][column] += left[row][index] * right[index][column]; }
        }
    }
    result
}

fn multiply_vector(matrix: [[f32; 3]; 3], vector: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|row| {
        matrix[row][0] * vector[0] + matrix[row][1] * vector[1] + matrix[row][2] * vector[2]
    })
}

fn add(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn normalize(value: [f32; 3]) -> [f32; 3] {
    let length = dot(value, value).sqrt();
    if length <= f32::EPSILON { [0.0, 1.0, 0.0] }
    else { [value[0] / length, value[1] / length, value[2] / length] }
}

pub fn draw_spawn_egg(ui: &mut egui::Ui, size: f32, selected: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter();
    let center = rect.center();
    let radius = size * 0.34;
    let mut outline = Vec::new();
    for index in 0..20 {
        let angle = std::f32::consts::TAU * index as f32 / 20.0;
        let y_scale = if angle.sin() < 0.0 { 0.82 } else { 1.08 };
        outline.push(center + egui::vec2(angle.cos() * radius * 0.72, angle.sin() * radius * y_scale));
    }
    painter.add(egui::Shape::convex_polygon(
        outline,
        Color32::from_rgb(230, 230, 218),
        egui::Stroke::new(1.0, if selected { Color32::WHITE } else { Color32::from_rgb(7, 9, 12) }),
    ));
    painter.circle_filled(center + egui::vec2(0.0, size * 0.05), size * 0.13, Color32::from_rgb(93, 153, 71));
    painter.circle_filled(center + egui::vec2(size * 0.08, -size * 0.09), size * 0.045, Color32::from_rgb(54, 112, 61));
    painter.circle_filled(center + egui::vec2(-size * 0.10, size * 0.11), size * 0.035, Color32::from_rgb(54, 112, 61));
}

#[cfg(test)]
mod tests {
    use super::*;
    use mojmodelbridge::viewer::{ViewerAnimation, ViewerNode};

    #[test]
    fn quad_uvs_follow_vanilla_textured_quad_vertex_order() {
        let points = [[0.0, 0.0, 0.0]; 4];
        let vertices = quad_from_rect(points, [1.0, 2.0, 3.0, 4.0], false, 16.0, 16.0);
        assert_eq!(vertices[0].uv, [3.0 / 16.0, 2.0 / 16.0]);
        assert_eq!(vertices[1].uv, [1.0 / 16.0, 2.0 / 16.0]);
        assert_eq!(vertices[2].uv, [1.0 / 16.0, 4.0 / 16.0]);
        assert_eq!(vertices[3].uv, [3.0 / 16.0, 4.0 / 16.0]);
    }

    #[test]
    fn rectangular_viewport_keeps_model_units_isotropic() {
        let camera = Camera {
            yaw: 0.0,
            pitch: 0.0,
            distance: 100.0,
            target: [0.0; 3],
            initialized: true,
        };
        let world = Transform::identity();
        let texture_size = [16, 16];
        let origin = MeshVertex { position: [0.0; 3], uv: [0.0; 2] };
        let x_unit = MeshVertex { position: [1.0, 0.0, 0.0], uv: [0.0; 2] };
        let y_unit = MeshVertex { position: [0.0, 1.0, 0.0], uv: [0.0; 2] };
        let project = |vertex| project_vertex(vertex, world, &camera, 160, 320, 232.0).unwrap();
        let origin = project(origin);
        let x = project(x_unit);
        let y = project(y_unit);
        assert!(((x.x - origin.x).abs() - (y.y - origin.y).abs()).abs() < 1e-5);
        let _ = texture_size;
    }

    #[test]
    fn software_viewport_draws_a_textured_scene_without_gpu_state() {
        let scene = ViewerScene {
            name: "TestCube".to_owned(),
            texture_size: [16, 16],
            texture: String::new(),
            texture_variants: Vec::new(),
            nodes: vec![ViewerNode {
                name: "body".to_owned(),
                parent: None,
                local_translation: [0.0, 12.0, 0.0],
                base_rotation: [0.0; 3],
                cubes: vec![ViewerCube {
                    min: [-4.0, -8.0, -4.0],
                    max: [4.0, 0.0, 4.0],
                    uv_origin: [0.0, 0.0],
                    face_uvs: None,
                    mirror: false,
                    name: "body_cube".to_owned(),
                }],
            }],
            animations: Vec::<ViewerAnimation>::new(),
            warnings: Vec::new(),
        };
        let pose = [NodePose {
            translation: [0.0, 12.0, 0.0],
            rotation: [0.0; 3],
        }];
        let mut camera = Camera::default();
        let rendered = render_scene(&scene, &pose, None, [160, 160], &mut camera);
        let background = Color32::from_rgb(22, 27, 38);
        assert!(rendered.pixels.iter().any(|pixel| *pixel != background));
    }
}
