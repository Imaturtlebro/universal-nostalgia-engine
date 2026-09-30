use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::render::settings::{Backends, PowerPreference, RenderCreation, WgpuSettings};
use bevy::render::RenderPlugin;
use bevy::window::{CursorGrabMode, PrimaryWindow};
use std::path::{Path, PathBuf};
use std::time::Instant;
use vpk::VPK;

const GMOD_ASSET_DIRS: [&str; 2] = [
    r"D:\GModAssets\garrysmod",
    r"D:\gmodassets\garrysmod",
];

const FLYCAM_SPEED: f32 = 5.0;
const FLYCAM_SENSITIVITY: f32 = 0.003;
const MAX_PITCH: f32 = std::f32::consts::FRAC_PI_2 * 0.99;

fn find_gmod_dir() -> Option<PathBuf> {
    GMOD_ASSET_DIRS
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_dir())
}

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
    let mut found: Option<PathBuf> = None;

    for candidate in GMOD_ASSET_DIRS.iter() {
        let path = Path::new(candidate);
        println!(
            "[asset-check] checking {:?} ... {}",
            candidate,
            if path.exists() {
                "FOUND"
            } else {
                "not found"
            }
        );

        if path.is_dir() {
            found = Some(path.to_path_buf());
            break;
        }
    }

    match found {
        Some(dir) => println!("[asset-check] Source engine assets located at {:?}", dir),
        None => println!(
            "[asset-check] WARNING: no garrysmod asset directory found. \
             Place your assets in D:\\GModAssets\\garrysmod to enable content loading."
        ),
    }
}

#[derive(Component)]
struct RotatableCube;

#[derive(Component, Default)]
struct Flycam {
    yaw: f32,
    pitch: f32,
}

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands
        .spawn((
            PbrBundle {
                mesh: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
                material: materials.add(Color::rgb(0.8, 0.2, 0.2)),
                transform: Transform::from_xyz(0.0, 0.5, 0.0),
                ..default()
            },
            RotatableCube,
        ));

    commands.spawn(PbrBundle {
        mesh: meshes.add(Plane3d::default().mesh().size(10.0, 10.0).build()),
        material: materials.add(Color::rgb(0.35, 0.35, 0.38)),
        transform: Transform::from_xyz(0.0, 0.0, 0.0),
        ..default()
    });

    // From (0, 2, 5), pitch is atan(-2/5) to aim at the origin.
    let spawn_position = Vec3::new(0.0, 2.0, 5.0);
    let spawn_pitch = -2.0_f32.atan2(5.0);

    commands.spawn((
        Camera3dBundle {
            transform: Transform::from_translation(spawn_position),
            ..default()
        },
        Flycam {
            yaw: 0.0,
            pitch: spawn_pitch,
        },
    ));

    commands.spawn(DirectionalLightBundle {
        directional_light: DirectionalLight {
            illuminance: 10_000.0,
            shadows_enabled: true,
            ..default()
        },
        transform: Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
        ..default()
    });
}

fn main() {
    println!("universal-nostalgia-engine {} starting", env!("CARGO_PKG_VERSION"));
    check_gmod_assets();
    inspect_vpk_archive();

    App::new()
        .insert_resource(ClearColor(Color::rgb(0.1, 0.2, 0.4)))
        .insert_resource(AmbientLight {
            color: Color::WHITE,
            brightness: 500.0,
        })
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
        .add_systems(Startup, setup_scene)
        .add_systems(Update, (rotate_cube, flycam_system).chain())
        .run();
}

fn rotate_cube(mut query: Query<&mut Transform, With<RotatableCube>>) {
    for mut transform in &mut query {
        transform.rotate_y(0.3 * 0.016);
        transform.rotate_x(0.2 * 0.016);
    }
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
