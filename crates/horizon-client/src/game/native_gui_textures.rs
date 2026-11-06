//! Mod-owned HUD textures. Register before batch preparation, then render into
//! the texture in `WorldPrePost`; the HUD samples it after the world passes.
use super::Game;
use horizon_client::hud::{
    HudBatch,
    native_textures::{
        NativeGuiNineSlice, NativeGuiTexture, NativeGuiTextureError, NativeGuiTextureFilter,
    },
};

#[allow(dead_code)]
impl Game<'_> {
    /// Register a texture from the current renderer's device before preparing
    /// its first GUI batch. Resource reload and device loss invalidate handles.
    /// API @API:gui_textures@.
    pub(super) fn native_gui_texture_register(
        &self,
        texture: &wgpu::Texture,
        filter: NativeGuiTextureFilter,
    ) -> Result<NativeGuiTexture, NativeGuiTextureError> {
        let renderer = self
            .renderer
            .as_ref()
            .ok_or(NativeGuiTextureError::RendererUnavailable)?;
        renderer
            .hud
            .native_gui_texture_register(&renderer.device, texture, filter)
    }

    /// Create a single-mip sampled render target with upload support before
    /// preparing its first GUI batch. API @API:gui_textures@.
    pub(super) fn native_gui_texture_create(
        &self,
        size: [u32; 2],
        format: wgpu::TextureFormat,
        filter: NativeGuiTextureFilter,
    ) -> Result<NativeGuiTexture, NativeGuiTextureError> {
        let renderer = self
            .renderer
            .as_ref()
            .ok_or(NativeGuiTextureError::RendererUnavailable)?;
        renderer
            .hud
            .native_gui_texture_create(&renderer.device, size, format, filter)
    }

    /// Append an ordered GUI draw in GUI units with normalized UVs and straight
    /// alpha ARGB tint. False means the handle or geometry is invalid; nothing
    /// is appended. API @API:gui_textures@.
    pub(super) fn native_gui_texture(
        &self,
        batch: &mut HudBatch,
        texture: &NativeGuiTexture,
        rect: [f32; 4],
        uv: [f32; 4],
        argb: u32,
        nine_slice: Option<NativeGuiNineSlice>,
    ) -> bool {
        self.renderer.as_ref().is_some_and(|renderer| {
            renderer.hud.native_gui_texture_owns(texture)
                && batch.native_gui_texture(texture, rect, uv, argb, nine_slice)
        })
    }
}
