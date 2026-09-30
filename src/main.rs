// bevy_diagnostic has no prelude module, so these must be imported explicitly.
use bevy::diagnostic::{Diagnostics, FrameTimeDiagnosticsPlugin};
use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::render::settings::{Backends, PowerPreference, RenderCreation, WgpuSettings};
use bevy::render::RenderPlugin;
use bevy::window::{CursorGrabMode, PrimaryWindow};
use std::path::Path;
use std::time::Instant;
use vpk::VPK;

mod bsp_world;
use bsp_world::{find_gmod_dir, LoadedMap};

/// Source units are ~inches (player is 72 units tall), so the flycam moves
/// much faster than a metre-scale Bevy scene would suggest.
const FLYCAM_SPEED: f32 = 250.0;
const FLYCAM_SENSITIVITY: f32 = 0.003;
const MAX_PITCH: f32 = std::f32::consts::FRAC_PI_2 * 0.99;
const AMBIENT_BRIGHTNESS: f32 = 250.0;

fn inspect_vpk_archive() {
    let Some(gmod_dir) = find_gmod_dir() else {
        println!("[vpk] no garrysmod directory found, skipping archive inspection");
        return;
    };

    let index_path = gmod_dir.join("garrysmod_dir.vpk");
    println!("[vpk] opening index {:?}", index_path);

    let started = Instant::now();
    let archive = match VPK::read(&index_path) {
        Ok(archive) => archive,
        Err(err) => {
            println!("[vpk] FAILED to read index: {:?}", err);
            return;
        }
    };

    println!(
        "[vpk] Mounted garrysmod_dir.vpk containing {} entries (parsed in {:?})",
        archive.tree.len(),
        started.elapsed()
    );

    let mut models: Vec<&String> = archive
        .tree
        .keys()
        .filter(|path| path.to_ascii_lowercase().ends_with(".mdl"))
        .collect();
    models.sort();

    println!(
        "[vpk] total .mdl entries in this index: {}",
        models.len()
    );
    println!("[vpk] first 10 model (.mdl) files inside the archive:");
    for path in models.iter().take(10) {
        println!("[vpk]   {}", path);
    }
}

fn check_gmod_assets() {
    for candidate in bsp_world::gmod_asset_dirs().iter() {
        println!(
            "[asset-check] checking {:?} ... {}",
            candidate,
            if Path::new(candidate).is_dir() {
                "FOUND"
            } else {
                "not found"
            }
        );
    }

    match find_gmod_dir() {
        Some(dir) => println!("[asset-check] Source engine assets located at {:?}", dir),
        None => println!(
            "[asset-check] WARNING: no garrysmod asset directory found. \
             Place your assets in D:\\GModAssets\\garrysmod to enable content loading."
        ),
    }
}

#[derive(Component, Default)]
struct Flycam {
    yaw: f32,
    pitch: f32,
}

/// Stand-in colour per texture name, so faces are visually distinguishable
/// before real .vtf textures are decoded. Replaced by actual textures later.
fn placeholder_color(texture_name: &str) -> Color {
    if texture_name.contains("grass") || texture_name.contains("dirt") {
        Color::rgb(0.25, 0.42, 0.16)
    } else if texture_name.contains("brick") {
        Color::rgb(0.48, 0.22, 0.16)
    } else if texture_name.contains("concrete") {
        Color::rgb(0.45, 0.45, 0.44)
    } else if texture_name.contains("plaster") || texture_name.contains("wall") {
        Color::rgb(0.76, 0.72, 0.64)
    } else if texture_name.contains("metal") {
        Color::rgb(0.40, 0.42, 0.46)
    } else if texture_name.contains("wood") {
        Color::rgb(0.44, 0.30, 0.18)
    } else if texture_name.contains("glass") {
        Color::rgb(0.60, 0.72, 0.80)
    } else {
        Color::rgb(0.62, 0.62, 0.65)
    }
}

fn spawn_camera(commands: &mut Commands, position: Vec3, yaw: f32, pitch: f32) {
    commands.spawn((
        Camera3dBundle {
            transform: Transform::from_translation(position),
            ..default()
        },
        Flycam { yaw, pitch },
    ));
}

/// Builds the loaded Source map if there is one, otherwise falls back to the
/// original grey-box test scene so the window is never empty.
fn setup_scene(
    mut commands: Commands,
    map: Option<Res<LoadedMap>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn(DirectionalLightBundle {
        directional_light: DirectionalLight {
            illuminance: 10_000.0,
            shadows_enabled: false,
            ..default()
        },
        transform: Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
        ..default()
    });

    if let Some(map) = map {
        // Backface culling is now safe because the mesh builder normalises
        // triangle winding against each face's plane normal.
        let materials: Vec<Handle<StandardMaterial>> = map
            .bucket_names
            .iter()
            .map(|name| {
                materials.add(StandardMaterial {
                    base_color: placeholder_color(name),
                    perceptual_roughness: 0.9,
                    ..default()
                })
            })
            .collect();

        for (bucket, chunk) in &map.chunks {
            commands.spawn(PbrBundle {
                mesh: meshes.add(chunk.clone()),
                material: materials[*bucket].clone(),
                ..default()
            });
        }

        spawn_camera(
            &mut commands,
            map.spawn_position,
            map.spawn_yaw,
            map.spawn_pitch,
        );
        return;
    }

    println!("[scene] no map loaded, falling back to the test scene");

    commands.spawn(PbrBundle {
        mesh: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        material: materials.add(Color::rgb(0.8, 0.2, 0.2)),
        transform: Transform::from_xyz(0.0, 0.5, 0.0),
        ..default()
    });

    commands.spawn(PbrBundle {
        mesh: meshes.add(Plane3d::default().mesh().size(10.0, 10.0).build()),
        material: materials.add(Color::rgb(0.35, 0.35, 0.38)),
        ..default()
    });

    // From (0, 2, 5), pitch is atan(-2/5) to aim at the origin.
    spawn_camera(
        &mut commands,
        Vec3::new(0.0, 2.0, 5.0),
        0.0,
        -2.0_f32.atan2(5.0),
    );
}

fn setup_fps_overlay(mut commands: Commands) {
    commands.spawn((
        // A plain text bundle has no background, so give it a dark panel to
        // stay readable against bright walls.
        NodeBundle {
            style: Style {
                position_type: PositionType::Absolute,
                top: Val::Px(8.0),
                left: Val::Px(8.0),
                padding: UiRect::all(Val::Px(6.0)),
                ..default()
            },
            background_color: Color::rgba(0.0, 0.0, 0.0, 0.55).into(),
            ..default()
        },
        TextBundle::from_section(
            "measuring...",
            TextStyle {
                font_size: 20.0,
                color: Color::WHITE,
                ..default()
            },
        ),
        FpsText,
    ));
}

#[derive(Component)]
struct FpsText;

#[derive(Resource, Default)]
struct RenderStats {
    /// Chunk meshes in the loaded map, known at load time.
    chunks_total: usize,
    triangles_total: usize,
}

fn update_fps_overlay(
    diagnostics: Diagnostics,
    mut fps_text: Query<&mut Text, With<FpsText>>,
    stats: Option<Res<RenderStats>>,
) {
    let Ok(mut text) = fps_text.get_single_mut() else {
        return;
    };

    let Some(diagnostic) = diagnostics.get(&FrameTimeDiagnosticsPlugin::FPS) else {
        return;
    };
    let Some(fps) = diagnostic.smoothed() else {
        return;
    };

    let mut line = format!("{fps:.1} fps");
    if let Some(stats) = stats {
        line.push_str(&format!(
            "  map chunks {}  drawn tris {}",
            stats.chunks_total,
            stats.triangles_total
        ));
    }
    text.sections[0].value = line;
}

/// The adapter/backend Bevy actually chose. Logged because the backend is
/// negotiated at runtime and can differ from what we requested.
fn report_render_backend(
    adapter: Res<bevy::render::renderer::RenderAdapterInfo>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    println!(
        "[perf] active backend: {:?} (vendor {:?}, device {:?}, driver {:?})",
        adapter.0.backend, adapter.0.vendor, adapter.0.device, adapter.0.driver
    );
    for window in &windows {
        println!(
            "[perf] primary window: {}x{} physical, scale {:?}",
            window.physical_width(),
            window.physical_height(),
            window.scale_factor()
        );
    }
}

/// One-line performance summary printed to the console a couple of seconds in.
fn log_startup_stats(diagnostics: Diagnostics, mut done: Local<bool>) {
    if *done {
        return;
    }

    let Some(diagnostic) = diagnostics.get(&FrameTimeDiagnosticsPlugin::FPS) else {
        return;
    };
    if diagnostic.history.len() < 120 {
        return;
    }
    *done = true;

    println!("[perf] ---- startup performance summary ----");
    if let Some(fps) = diagnostic.average() {
        println!("[perf] average fps over first ~2s: {fps:.1}");
    }
    if let Some(frame_time) = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|d| d.average())
    {
        println!("[perf] average frame time: {frame_time:.2} ms");
    }
}

fn main() {
    println!("universal-nostalgia-engine {} starting", env!("CARGO_PKG_VERSION"));
    check_gmod_assets();
    inspect_vpk_archive();

    let map = bsp_world::load_map();
    if map.is_none() {
        println!("[bsp] map load failed, continuing without a world");
    }

    let mut app = App::new();
    app.insert_resource(ClearColor(Color::rgb(0.1, 0.2, 0.4)))
        // MSAA off: in Bevy 0.13 Msaa is a Resource, not a camera component.
        // 4x multisampling costs fill rate the Intel UHD 600 cannot spare.
        .insert_resource(Msaa::Off)
        .insert_resource(AmbientLight {
            color: Color::WHITE,
            brightness: AMBIENT_BRIGHTNESS,
        });

    if let Some(map) = map {
        println!(
            "[scene] world ready: {} chunk meshes, {} triangles, {} material buckets",
            map.chunks.len(),
            map.triangles_total,
            map.bucket_names.len()
        );
        app.insert_resource(RenderStats {
            chunks_total: map.chunks.len(),
            triangles_total: map.triangles_total,
        });
        app.insert_resource(map);
    }

    app.add_plugins(FrameTimeDiagnosticsPlugin::default())
        .add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Universal Nostalgia Engine".to_string(),
                    resolution: (1280.0_f32, 720.0_f32).into(),
                    ..default()
                }),
                ..default()
            })
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(WgpuSettings {
                    backends: Some(Backends::DX12 | Backends::VULKAN),
                    power_preference: PowerPreference::HighPerformance,
                    ..default()
                }),
                synchronous_pipeline_compilation: true,
            }),
        )
        .add_systems(
            Startup,
            (
                setup_scene,
                setup_fps_overlay,
                report_render_backend,
            )
                .chain(),
        )
        .add_systems(
            Update,
            (
                update_fps_overlay,
                log_startup_stats,
                flycam_system,
            )
                .chain(),
        )
        .run();
}

fn flycam_system(
    time: Res<Time<()>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut mouse_motion: EventReader<MouseMotion>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut cameras: Query<(&mut Transform, &mut Flycam), With<Camera3d>>,
) {
    for mut window in &mut windows {
        let release = mouse_buttons.just_released(MouseButton::Right)
            || keyboard.just_pressed(KeyCode::Escape);

        if release {
            window.cursor.grab_mode = CursorGrabMode::None;
            window.cursor.visible = true;
        }

        if mouse_buttons.just_pressed(MouseButton::Right) {
            window.cursor.grab_mode = CursorGrabMode::Locked;
            window.cursor.visible = false;
        }
    }

    let look_enabled = mouse_buttons.pressed(MouseButton::Right);
    let mut motion = Vec2::ZERO;
    for event in mouse_motion.read() {
        motion += event.delta;
    }
    if !look_enabled {
        motion = Vec2::ZERO;
    }

    let mut direction = Vec3::ZERO;
    if keyboard.pressed(KeyCode::KeyW) {
        direction += Vec3::Z;
    }
    if keyboard.pressed(KeyCode::KeyS) {
        direction -= Vec3::Z;
    }
    if keyboard.pressed(KeyCode::KeyA) {
        direction -= Vec3::X;
    }
    if keyboard.pressed(KeyCode::KeyD) {
        direction += Vec3::X;
    }
    if keyboard.pressed(KeyCode::Space) {
        direction += Vec3::Y;
    }
    if keyboard.pressed(KeyCode::ShiftLeft) || keyboard.pressed(KeyCode::ShiftRight) {
        direction -= Vec3::Y;
    }

    for (mut transform, mut flycam) in &mut cameras {
        if motion != Vec2::ZERO {
            flycam.yaw -= motion.x * FLYCAM_SENSITIVITY;
            flycam.pitch = (flycam.pitch - motion.y * FLYCAM_SENSITIVITY)
                .clamp(-MAX_PITCH, MAX_PITCH);
        }

        // Rebuilt from yaw/pitch every frame, so roll can never accumulate.
        transform.rotation =
            Quat::from_euler(EulerRot::YXZ, flycam.yaw, flycam.pitch, 0.0);

        if direction != Vec3::ZERO {
            let forward = transform.forward();
            let right = transform.right();
            let motion_direction =
                forward * direction.z + right * direction.x + Vec3::Y * direction.y;

            transform.translation += motion_direction.normalize_or_zero()
                * FLYCAM_SPEED
                * time.delta_seconds();
        }
    }
}
