//! Spawner menu over the entities stored in the BSP.
//!
//! Scope note: these are the map's *entities* (`prop_physics`, `func_door`,
//! lights, triggers, ...), read from LUMP_ENTITIES, which `vbsp` already
//! parses. They are not rendered as GMod models: drawing an actual `.mdl` prop
//! needs a Source Studio model parser, a VTF skin decoder and a skinned mesh
//! pipeline, none of which exist yet. So the menu places debug markers and
//! teleports, which is what makes the map explorable today.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow};

use crate::player::Player;

/// Entities that are map dressing or the world itself, not spawnable content.
const IGNORED_CLASSES: [&str; 8] = [
    "worldspawn",
    "light",
    "light_environment",
    "info_player_start",
    "info_landmark",
    "info_notarget",
    "trigger_",
    "func_",
];

/// Cap on the menu so it stays navigable and the console log stays readable.
const MAX_ENTITIES: usize = 400;

#[derive(Debug, Clone)]
pub struct Spawnable {
    pub classname: String,
    pub origin: Vec3,
    pub model: String,
}

#[derive(Resource, Default)]
pub struct Spawner {
    pub entities: Vec<Spawnable>,
    pub selected: usize,
    /// Scroll offset so a long list can be paged.
    pub window: usize,
    pub open: bool,
    pub spawned: u32,
}

#[derive(Component)]
pub struct MenuText;

#[derive(Component)]
pub struct MenuHint;

#[derive(Component)]
pub struct SpawnedMarker;

/// Reads the entity lump and builds the spawnable list.
pub fn collect_spawnables(bsp: &vbsp::Bsp) -> Vec<Spawnable> {
    let mut spawnables: Vec<Spawnable> = Vec::new();

    for entity in bsp.entities.iter() {
        let Some(classname) = entity.prop("classname") else {
            continue;
        };

        if IGNORED_CLASSES
            .iter()
            .any(|ignored| classname.starts_with(ignored))
        {
            continue;
        }

        let Some(origin) = entity.prop("origin").and_then(parse_vec3) else {
            continue;
        };

        spawnables.push(Spawnable {
            classname: classname.to_ascii_lowercase(),
            // Entity origins are in Source coordinates, like the geometry.
            origin: Vec3::new(origin[0], origin[2], -origin[1]),
            model: entity.prop("model").unwrap_or_default().to_string(),
        });

        if spawnables.len() >= MAX_ENTITIES {
            break;
        }
    }

    // Group by class so the list reads as a catalogue rather than map order.
    spawnables.sort_by(|a, b| a.classname.cmp(&b.classname));
    spawnables
}

fn parse_vec3(raw: &str) -> Option<[f32; 3]> {
    let mut values = [0.0_f32; 3];
    for (slot, part) in values.iter_mut().zip(raw.split_whitespace()) {
        *slot = part.parse().ok()?;
    }
    Some(values)
}

/// Spawns the menu panel. Hidden until the user opens it.
pub fn setup_menu(mut commands: Commands) {
    commands.spawn((
        NodeBundle {
            style: Style {
                position_type: PositionType::Absolute,
                top: Val::Px(40.0),
                left: Val::Px(8.0),
                width: Val::Px(420.0),
                max_height: Val::Px(560.0),
                padding: UiRect::all(Val::Px(8.0)),
                flex_direction: FlexDirection::Column,
                overflow: Overflow::clip(),
                ..default()
            },
            background_color: Color::rgba(0.02, 0.02, 0.04, 0.88).into(),
            visibility: Visibility::Hidden,
            ..default()
        },
        MenuText,
    ));

    commands.spawn((
        NodeBundle {
            style: Style {
                position_type: PositionType::Absolute,
                top: Val::Px(12.0),
                left: Val::Px(8.0),
                padding: UiRect::all(Val::Px(5.0)),
                ..default()
            },
            background_color: Color::rgba(0.0, 0.0, 0.0, 0.6).into(),
            visibility: Visibility::Hidden,
            ..default()
        },
        MenuHint,
    ));
}

/// Menu navigation, spawning and teleporting.
pub fn menu_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut spawner: ResMut<Spawner>,
    mut players: Query<&mut Player>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    markers: Query<Entity, With<SpawnedMarker>>,
) {
    if spawner.entities.is_empty() {
        return;
    }

    if keyboard.just_pressed(KeyCode::KeyE) {
        spawner.open = !spawner.open;
        for mut window in &mut windows {
            window.cursor.grab_mode = if spawner.open {
                CursorGrabMode::None
            } else {
                CursorGrabMode::Locked
            };
            window.cursor.visible = !spawner.open;
        }
    }

    if !spawner.open {
        return;
    }

    if keyboard.just_pressed(KeyCode::ArrowDown) || keyboard.just_pressed(KeyCode::KeyS) {
        spawner.selected = (spawner.selected + 1).min(spawner.entities.len() - 1);
    }
    if keyboard.just_pressed(KeyCode::ArrowUp) || keyboard.just_pressed(KeyCode::KeyW) {
        spawner.selected = spawner.selected.saturating_sub(1);
    }

    // Keep the selection inside the visible page.
    if spawner.selected < spawner.window {
        spawner.window = spawner.selected;
    } else if spawner.selected >= spawner.window + LIST_HEIGHT {
        spawner.window = spawner.selected + 1 - LIST_HEIGHT;
    }

    // Enter drops a marker at the entity so it can be found in the world.
    if keyboard.just_pressed(KeyCode::Enter) {
        let Some(entity) = spawner.entities.get(spawner.selected) else {
            return;
        };

        let handle = meshes.add(Cuboid::new(MARKER_SIZE, MARKER_SIZE, MARKER_SIZE));
        let material = materials.add(StandardMaterial {
            base_color: Color::rgba(1.0, 0.55, 0.1, 1.0),
            emissive: Color::rgb(0.6, 0.25, 0.0),
            ..default()
        });

        commands.spawn((
            PbrBundle {
                mesh: handle,
                material,
                transform: Transform::from_translation(entity.origin + Vec3::Y * 24.0),
                ..default()
            },
            SpawnedMarker,
        ));
        spawner.spawned += 1;
    }

    // T walks to the selected entity instead of dropping a marker.
    if keyboard.just_pressed(KeyCode::KeyT) {
        if let Some(entity) = spawner.entities.get(spawner.selected) {
            for mut player in &mut players {
                player.feet = entity.origin;
                player.velocity = Vec3::ZERO;
            }
        }
    }

    // Backspace clears every marker placed so far.
    if keyboard.just_pressed(KeyCode::Backspace) {
        for marker in &markers {
            commands.entity(marker).despawn_recursive();
        }
        spawner.spawned = 0;
    }

    // Keep the camera glued to the player when teleporting.
    for mut camera in &mut cameras {
        if let Ok(mut player) = players.single() {
            camera.translation = player.feet + Vec3::Y * player.eye_height();
        }
    }
}

const LIST_HEIGHT: usize = 18;
const MARKER_SIZE: f32 = 24.0;

/// Redraws the menu panel and the hint line.
pub fn update_menu_text(
    keyboard: Res<ButtonInput<KeyCode>>,
    spawner: Res<Spawner>,
    mut menu: Query<&mut Text, With<MenuText>>,
    mut hint: Query<(&mut Text, &mut Visibility), With<MenuHint>>,
    mut menu_visibility: Query<&mut Visibility, (With<MenuText>, Without<MenuHint>)>,
) {
    let Ok(mut text) = menu.get_single_mut() else {
        return;
    };

    if let Ok((mut hint_text, mut hint_visibility)) = hint.get_single_mut() {
        *hint_visibility = if spawner.open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        hint_text.sections[0].value = if spawner.open {
            "E close  W/S or arrows select  ENTER drop marker  T teleport  BKSP clear".to_string()
        } else {
            "E open spawner menu".to_string()
        };
    }

    for mut visibility in &mut menu_visibility {
        *visibility = if spawner.open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }

    if !spawner.open {
        return;
    }

    let start = spawner.window.min(spawner.entities.len().saturating_sub(1));
    let end = (start + LIST_HEIGHT).min(spawner.entities.len());

    let mut line = format!(
        "SPAWNER  {} entities  markers: {}\n",
        spawner.entities.len(),
        spawner.spawned
    );

    for index in start..end {
        let Some(entity) = spawner.entities.get(index) else {
            continue;
        };
        let marker = if index == spawner.selected { ">" } else { " " };
        let short_model = shorten_model(&entity.model);
        line.push_str(&format!(
            "{marker} {:<28} {:>7.0},{:>7.0},{:>7.0}  {short_model}\n",
            entity.classname,
            entity.origin.x,
            entity.origin.y,
            entity.origin.z
        ));
    }

    if spawner.entities.len() > LIST_HEIGHT {
        line.push_str(&format!(
            "  {}-{} of {}",
            start + 1,
            end,
            spawner.entities.len()
        ));
    }

    text.sections[0].value = line.trim_end().to_string();
    let _ = keyboard;
}

/// `models/props/foo/bar.mdl` -> `bar.mdl`, so the list stays narrow.
fn shorten_model(model: &str) -> &str {
    if model.is_empty() {
        return "-";
    }
    model.rsplit('/').next().unwrap_or(model)
}
