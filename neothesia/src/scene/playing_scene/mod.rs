use midi_file::midly::MidiMessage;
use neothesia_core::render::{GlowInstance, GlowPipeline, QuadInstance, QuadPipeline};
use std::time::Duration;
use wgpu_jumpstart::{Color, TransformUniform, Uniform};
use winit::event::{KeyboardInput, WindowEvent};

use super::Scene;
use crate::{
    render::WaterfallRenderer, song::Song, target::Target, utils::window::WindowState,
    NeothesiaEvent,
};

mod keyboard;
use keyboard::Keyboard;

mod midi_player;
use midi_player::MidiPlayer;

mod rewind_controller;
use rewind_controller::RewindController;

mod toast_manager;
use toast_manager::ToastManager;

struct GlowState {
    time: f32,
}

pub struct PlayingScene {
    keyboard: Keyboard,
    notes: WaterfallRenderer,

    player: MidiPlayer,
    rewind_controler: RewindController,
    quad_pipeline: QuadPipeline,
    glow_pipeline: GlowPipeline,
    glow_states: Vec<GlowState>,
    toast_manager: ToastManager,
}

impl PlayingScene {
    pub fn new(target: &Target, song: Song) -> Self {
        let keyboard = Keyboard::new(target, song.config.clone());

        let keyboard_layout = keyboard.layout();

        let hidden_tracks: Vec<usize> = song
            .config
            .tracks
            .iter()
            .filter(|t| !t.visible)
            .map(|t| t.track_id)
            .collect();

        let mut notes = WaterfallRenderer::new(
            &target.gpu,
            &song.file.tracks,
            &hidden_tracks,
            &target.config,
            &target.transform,
            keyboard_layout.clone(),
        );

        let player = MidiPlayer::new(target, song, keyboard_layout.range.clone());
        notes.update(&target.gpu.queue, player.time_without_lead_in());

        let glow_states: Vec<GlowState> = keyboard
            .layout()
            .range
            .iter()
            .map(|_| GlowState { time: 0.0 })
            .collect();

        Self {
            keyboard,

            notes,
            player,
            rewind_controler: RewindController::new(),
            quad_pipeline: QuadPipeline::new(&target.gpu, &target.transform),
            glow_pipeline: GlowPipeline::new(&target.gpu, &target.transform),
            glow_states,
            toast_manager: ToastManager::default(),
        }
    }

    fn update_progresbar(&mut self, window_state: &WindowState) {
        let size_x = window_state.logical_size.width * self.player.percentage();
        self.quad_pipeline.instances().push(QuadInstance {
            position: [0.0, 0.0],
            size: [size_x, 5.0],
            color: Color::from_rgba8(56, 145, 255, 1.0).into_linear_rgba(),
            ..Default::default()
        });
    }

    fn update_glow(&mut self) {
        self.glow_pipeline.clear();

        let key_states = self.keyboard.key_states();
        for key in self.keyboard.layout().keys.iter() {
            let glow_state = &mut self.glow_states[key.id()];
            let glow_w = 200.0 + glow_state.time.sin() * 50.0;
            let glow_h = 200.0 + glow_state.time.sin() * 50.0;

            let y = self.keyboard.pos().y;
            if let Some(color) = &key_states[key.id()].pressed_by_file {
                glow_state.time += 0.1;
                let mut color = color.into_linear_rgba();
                color[0] += 0.1;
                color[1] += 0.1;
                color[2] += 0.1;
                color[3] = 0.3;
                self.glow_pipeline.instances().push(GlowInstance {
                    position: [key.x() - glow_w / 2.0 + key.width() / 2.0, y - glow_w / 2.0],
                    size: [glow_w, glow_h],
                    color,
                });
            }
        }
    }
}

impl Scene for PlayingScene {
    fn resize(&mut self, target: &mut Target) {
        self.keyboard.resize(target);
        self.notes.resize(
            &target.gpu.queue,
            &target.config,
            self.keyboard.layout().clone(),
        );
    }

    fn update(&mut self, target: &mut Target, delta: Duration) {
        self.rewind_controler.update(&mut self.player, target);

        if self.player.play_along().are_required_keys_pressed() {
            let midi_events = self.player.update(target, delta);
            self.keyboard.file_midi_events(&target.config, &midi_events);
        }

        self.toast_manager.update(&mut target.text_renderer);

        self.notes.update(
            &target.gpu.queue,
            self.player.time_without_lead_in() + target.config.playback_offset,
        );

        self.quad_pipeline.clear();

        self.keyboard
            .update(&mut self.quad_pipeline, &mut target.text_renderer);

        self.update_progresbar(&target.window_state);
        self.update_glow();

        self.quad_pipeline.prepare(&target.gpu.queue);
        self.glow_pipeline.prepare(&target.gpu.queue);
    }

    fn render<'pass>(
        &'pass mut self,
        transform: &'pass Uniform<TransformUniform>,
        rpass: &mut wgpu::RenderPass<'pass>,
    ) {
        self.notes.render(transform, rpass);
        self.quad_pipeline.render(transform, rpass);
        self.glow_pipeline.render(transform, rpass);
    }

    fn window_event(&mut self, target: &mut Target, event: &WindowEvent) {
        use winit::event::WindowEvent::*;
        use winit::event::{ElementState, VirtualKeyCode};

        match &event {
            KeyboardInput { input, .. } => {
                self.rewind_controler
                    .handle_keyboard_input(&mut self.player, input);

                if self.rewind_controler.is_rewinding() {
                    self.keyboard.reset_notes();
                }

                settings_keyboard_input(target, &mut self.toast_manager, input, &mut self.notes);

                if input.state == ElementState::Released {
                    match input.virtual_keycode {
                        Some(VirtualKeyCode::Escape) => {
                            target.proxy.send_event(NeothesiaEvent::MainMenu).ok();
                        }
                        Some(VirtualKeyCode::Space) => {
                            self.player.pause_resume();
                        }
                        _ => {}
                    }
                }
            }
            MouseInput { state, button, .. } => {
                self.rewind_controler.handle_mouse_input(
                    &mut self.player,
                    &target.window_state,
                    state,
                    button,
                );

                if self.rewind_controler.is_rewinding() {
                    self.keyboard.reset_notes();
                }
            }
            CursorMoved { position, .. } => {
                self.rewind_controler.handle_cursor_moved(
                    &mut self.player,
                    &target.window_state,
                    position,
                );
            }
            _ => {}
        }
    }

    fn midi_event(&mut self, _target: &mut Target, _channel: u8, message: &MidiMessage) {
        self.player
            .play_along_mut()
            .midi_event(midi_player::MidiEventSource::User, message);
        self.keyboard.user_midi_event(message);
    }
}

fn settings_keyboard_input(
    target: &mut Target,
    toast_manager: &mut ToastManager,
    input: &KeyboardInput,
    waterfall: &mut WaterfallRenderer,
) {
    use winit::event::{ElementState, VirtualKeyCode};

    if input.state != ElementState::Released {
        return;
    }

    let virtual_keycode = if let Some(virtual_keycode) = input.virtual_keycode {
        virtual_keycode
    } else {
        return;
    };

    match virtual_keycode {
        VirtualKeyCode::Up | VirtualKeyCode::Down => {
            let amount = if target.window_state.modifers_state.shift() {
                0.5
            } else {
                0.1
            };

            if virtual_keycode == VirtualKeyCode::Up {
                target.config.speed_multiplier += amount;
            } else {
                target.config.speed_multiplier -= amount;
                target.config.speed_multiplier = target.config.speed_multiplier.max(0.0);
            }

            toast_manager.speed_toast(target.config.speed_multiplier);
        }

        VirtualKeyCode::PageUp | VirtualKeyCode::PageDown => {
            let amount = if target.window_state.modifers_state.shift() {
                500.0
            } else {
                100.0
            };

            if virtual_keycode == VirtualKeyCode::PageUp {
                target.config.animation_speed += amount;
            } else {
                target.config.animation_speed -= amount;
                target.config.animation_speed = target.config.animation_speed.max(100.0);
            }

            waterfall
                .pipeline()
                .set_speed(&target.gpu.queue, target.config.animation_speed);
            toast_manager.animation_speed_toast(target.config.animation_speed);
        }

        VirtualKeyCode::Minus | VirtualKeyCode::Plus | VirtualKeyCode::Equals => {
            let amount = if target.window_state.modifers_state.shift() {
                0.1
            } else {
                0.01
            };

            if virtual_keycode == VirtualKeyCode::Minus {
                target.config.playback_offset -= amount;
            } else {
                target.config.playback_offset += amount;
            }

            toast_manager.offset_toast(target.config.playback_offset);
        }

        _ => {}
    }
}
