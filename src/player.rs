//! Grounded first-person controller: walking, running, crouching, jumping,
//! strafing and sliding, plus the camera bob, sway and lean that sell it.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow};

use crate::collision::{
    build_collision_world, CollisionWorld, CROUCH_EYE_HEIGHT, CROUCH_HEIGHT, EYE_HEIGHT,
    PLAYER_HEIGHT, PLAYER_RADIUS, STEP_HEIGHT,
};

const WALK_SPEED: f32 = 150.0;
const RUN_SPEED: f32 = 250.0;
const CROUCH_SPEED: f32 = 90.0;
const SLIDE_SPEED: f32 = 420.0;
const GROUND_ACCELERATION: f32 = 900.0;
const AIR_ACCELERATION: f32 = 250.0;
const GROUND_FRICTION: f32 = 900.0;

const JUMP_SPEED: f32 = 300.0;
const GRAVITY: f32 = 800.0;

/// Downward speed below which the player is treated as standing on the floor,
/// so a jump apex does not flicker the grounded state.
const GROUND_EPSILON: f32 = 20.0;

/// How long a slide lasts before it is cut short, in seconds.
const SLIDE_TIME: f32 = 0.9;
const SLIDE_COOLDOWN: f32 = 0.6;

/// Head bob amplitude and frequency, tuned against WALK_SPEED.
const BOB_AMPLITUDE: f32 = 1.6;
const BOB_FREQUENCY: f32 = 0.055;
const BOB_LATERAL_AMPLITUDE: f32 = 1.0;

/// Maximum camera roll from strafing, in degrees.
const LEAN_MAX_DEGREES: f32 = 1.6;
const LEAN_SPEED: f32 = 4.0;

#[derive(Resource)]
pub struct Collision(pub CollisionWorld);

#[derive(Component)]
pub struct Player {
    pub feet: Vec3,
    pub velocity: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub height: f32,
    pub grounded: bool,
    pub crouching: bool,
    pub sliding: f32,
    pub slide_cooldown: f32,
    /// Distance walked, drives the bob phase.
    pub bob_phase: f32,
    pub lean: f32,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            feet: Vec3::ZERO,
            velocity: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            height: PLAYER_HEIGHT,
            grounded: false,
            crouching: false,
            sliding: 0.0,
            slide_cooldown: 0.0,
            bob_phase: 0.0,
            lean: 0.0,
        }
    }
}

impl Player {
    pub fn eye_height(&self) -> f32 {
        if self.crouching {
            CROUCH_EYE_HEIGHT
        } else {
            EYE_HEIGHT
        }
    }

    pub fn head(&self) -> Vec3 {
        self.feet + Vec3::Y * self.height
    }
}

/// Reads mouse look while the right mouse button is held, and toggles cursor
/// grab. Kept separate from movement so camera orientation is always applied
/// even while the controller is busy.
pub fn look_system(
    time: Res<Time<()>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut mouse_motion: EventReader<bevy::input::mouse::MouseMotion>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut players: Query<&mut Player>,
) {
    for mut window in &mut windows {
        if mouse_buttons.just_pressed(MouseButton::Right) {
            window.cursor.grab_mode = CursorGrabMode::Locked;
            window.cursor.visible = false;
        }
        if mouse_buttons.just_released(MouseButton::Right)
            || keyboard.just_pressed(KeyCode::Escape)
        {
            window.cursor.grab_mode = CursorGrabMode::None;
            window.cursor.visible = true;
        }
    }

    if !mouse_buttons.pressed(MouseButton::Right) {
        return;
    }

    let mut motion = Vec2::ZERO;
    for event in mouse_motion.read() {
        motion += event.delta;
    }
    if motion == Vec2::ZERO {
        return;
    }

    let sensitivity = 0.003;
    for mut player in &mut players {
        player.yaw -= motion.x * sensitivity;
        player.pitch = (player.pitch - motion.y * sensitivity)
            .clamp(-std::f32::consts::FRAC_PI_2 * 0.99, std::f32::consts::FRAC_PI_2 * 0.99);
    }
    let _ = time;
}

/// The grounded first-person controller.
pub fn move_system(
    time: Res<Time<()>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    collision: Option<Res<Collision>>,
    mut players: Query<&mut Player>,
) {
    let delta = time.delta_seconds();
    if delta <= 0.0 {
        return;
    }

    // Without a map there is no world to stand in; the fallback scene has no
    // collision, so leave the player where they are rather than letting
    // gravity apply and never resolve.
    let Some(collision) = collision else {
        return;
    };
    let collision = collision.into_inner();

    for mut player in &mut players {
        let mut wish = Vec3::ZERO;
        if keyboard.pressed(KeyCode::KeyW) {
            wish += Vec3::Z;
        }
        if keyboard.pressed(KeyCode::KeyS) {
            wish -= Vec3::Z;
        }
        if keyboard.pressed(KeyCode::KeyA) {
            wish -= Vec3::X;
        }
        if keyboard.pressed(KeyCode::KeyD) {
            wish += Vec3::X;
        }
        let moving = wish != Vec3::ZERO;

        player.slide_cooldown = (player.slide_cooldown - delta).max(0.0);
        player.sliding = (player.sliding - delta).max(0.0);

        // Crouch, and stand back up only when there is headroom.
        if keyboard.just_pressed(KeyCode::ControlLeft) {
            player.crouching = !player.crouching;
        }
        if !player.crouching && !has_headroom(&collision, &player) {
            player.crouching = true;
        }
        player.height = if player.crouching {
            CROUCH_HEIGHT
        } else {
            PLAYER_HEIGHT
        };

        // Slide: crouch while sprinting, then keep the momentum.
        if player.slide_cooldown <= 0.0
            && player.grounded
            && player.crouching
            && keyboard.pressed(KeyCode::ShiftLeft)
            && player.velocity.length() > WALK_SPEED
        {
            player.sliding = SLIDE_TIME;
            player.slide_cooldown = SLIDE_TIME + SLIDE_COOLDOWN;
            let horizontal = Vec3::new(player.velocity.x, 0.0, player.velocity.z);
            let direction = if horizontal.length() > 0.0 {
                horizontal.normalize()
            } else {
                forward(&player.yaw)
            };
            player.velocity.x = direction.x * SLIDE_SPEED;
            player.velocity.z = direction.z * SLIDE_SPEED;
        }

        let forward = forward(&player.yaw);
        let right = Vec3::new(forward.z, 0.0, -forward.x);

        let target_speed = if player.sliding > 0.0 {
            SLIDE_SPEED
        } else if player.crouching {
            CROUCH_SPEED
        } else if keyboard.pressed(KeyCode::ShiftLeft) {
            RUN_SPEED
        } else {
            WALK_SPEED
        };

        let wish_direction =
            (forward * wish.z + right * wish.x).normalize_or_zero();

        let horizontal = Vec3::new(player.velocity.x, 0.0, player.velocity.z);
        let acceleration = if player.grounded {
            GROUND_ACCELERATION
        } else {
            AIR_ACCELERATION
        };

        if player.sliding > 0.0 {
            // Steering is weak while sliding.
            player.velocity.x += wish_direction.x * acceleration * 0.25 * delta;
            player.velocity.z += wish_direction.z * acceleration * 0.25 * delta;
        } else {
            player.velocity.x =
                approach(player.velocity.x, wish_direction.x * target_speed, acceleration * delta);
            player.velocity.z =
                approach(player.velocity.z, wish_direction.z * target_speed, acceleration * delta);

            if player.grounded && !moving {
                player.velocity.x =
                    approach(player.velocity.x, 0.0, GROUND_FRICTION * delta);
                player.velocity.z =
                    approach(player.velocity.z, 0.0, GROUND_FRICTION * delta);
            }
        }

        // Lean into strafes, for the camera roll only.
        let strafe = wish.x;
        let lean_target = strafe * LEAN_MAX_DEGREES.to_radians();
        player.lean += (lean_target - player.lean) * (LEAN_SPEED * delta).min(1.0);

        if keyboard.just_pressed(KeyCode::Space) && player.grounded {
            player.velocity.y = JUMP_SPEED;
            player.grounded = false;
        }

        player.velocity.y -= GRAVITY * delta;
        if player.velocity.y < -2000.0 {
            player.velocity.y = -2000.0;
        }

        // Move in three passes so landing on a ledge does not lose the step.
        let before = player.feet;
        player.feet += player.velocity * delta;

        let (resolved, contact) = collision.resolve(player.feet, player.head(), PLAYER_RADIUS);

        // Step up over low obstacles rather than stopping dead against them.
        if let Some(normal) = contact {
            if normal.y < 0.5 && player.grounded {
                let lifted = resolved + Vec3::Y * STEP_HEIGHT;
                let (stepped, step_contact) =
                    collision.resolve(lifted, lifted + Vec3::Y * player.height, PLAYER_RADIUS);
                let still_blocked = step_contact.map(|n| n.y < 0.5).unwrap_or(false);
                if !still_blocked {
                    player.feet = stepped - Vec3::Y * STEP_HEIGHT;
                } else {
                    player.feet = resolved;
                }
            } else {
                player.feet = resolved;
            }
        }

        // Ground contact: the contact normal points up out of the surface.
        player.grounded = contact.map(|normal| normal.y > 0.7).unwrap_or(false) && player.velocity.y <= 0.0;
        if player.grounded && player.velocity.y < 0.0 {
            player.velocity.y = 0.0;
        }

        let travelled = player.feet - before;
        let horizontal_travel =
            (travelled.x * travelled.x + travelled.z * travelled.z).sqrt();
        if player.grounded {
            player.bob_phase += horizontal_travel * BOB_FREQUENCY;
        }
    }
}

/// Applies the player's orientation and the camera bob/sway/lean offsets to the
/// camera entity, keeping it rigidly attached to the player's eyes.
pub fn camera_system(
    mut players: Query<&mut Player>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
) {
    for player in &players {
        for mut camera in &mut cameras {
            // Sway lags the look direction, so fast turns rock the view.
            let bob = if player.grounded {
                Vec3::new(
                    player.bob_phase.sin() * BOB_AMPLITUDE,
                    player.bob_phase.cos() * BOB_AMPLITUDE * 2.0,
                    0.0,
                )
            } else {
                Vec3::ZERO
            };
            let sway = Vec3::new(
                (player.bob_phase * 0.5).cos() * BOB_LATERAL_AMPLITUDE,
                0.0,
                0.0,
            );

            let eye = player.feet + Vec3::Y * player.eye_height() + bob + sway;
            camera.translation = eye;

            camera.rotation = Quat::from_euler(
                EulerRot::YXZ,
                player.yaw,
                player.pitch,
                // Roll from strafing.
                player.lean,
            );
        }
    }
}

fn forward(yaw: f32) -> Vec3 {
    Vec3::new(-yaw.sin(), 0.0, -yaw.cos())
}

/// Whether the player would fit standing up at its current position.
fn has_headroom(collision: &CollisionWorld, player: &Player) -> bool {
    let feet = player.feet + Vec3::Y * (CROUCH_HEIGHT + 1.0);
    let head = feet + Vec3::Y * (PLAYER_HEIGHT - CROUCH_HEIGHT);
    let (resolved, _) = collision.resolve(feet, head, PLAYER_RADIUS * 0.9);
    resolved.distance(head) < 1.0
}

/// Moves `current` towards `target` by at most `max_delta`.
fn approach(current: f32, target: f32, max_delta: f32) -> f32 {
    let difference = target - current;
    if difference.abs() <= max_delta {
        target
    } else {
        current + difference.signum() * max_delta
    }
}

/// Spawns the collision world resource. Called from `main` before the app runs
/// because building it needs the parsed BSP.
pub fn build_collision_resource(bsp: &vbsp::Bsp) -> Collision {
    let world = build_collision_world(bsp);
    println!(
        "[collision] built {} solid brushes for player movement",
        world.brush_count()
    );
    Collision(world)
}

