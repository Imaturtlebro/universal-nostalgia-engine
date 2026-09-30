use bevy::prelude::*;
use bevy::render::settings::{Backends, PowerPreference, RenderCreation, WgpuSettings};
use bevy::render::RenderPlugin;
use std::path::{Path, PathBuf};

const GMOD_ASSET_DIRS: [&str; 2] = [
    r"D:\GModAssets\garrysmod",
    r"D:\gmodassets\garrysmod",
];

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

    commands.spawn(Camera3dBundle {
        transform: Transform::from_xyz(0.0, 2.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        ..default()
    });

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
        .add_systems(Update, rotate_cube)
        .run();
}

fn rotate_cube(mut query: Query<&mut Transform, With<RotatableCube>>) {
    for mut transform in &mut query {
        transform.rotate_y(0.3 * 0.016);
        transform.rotate_x(0.2 * 0.016);
    }
}
