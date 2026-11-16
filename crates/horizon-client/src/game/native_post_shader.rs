use super::Game;
use horizon_client::gfx::native_post_shader::NativePostUniformLayout;

fn owned_shader(id: &str) -> Result<(), String> {
    if !super::native_key_bindings::owned_binding(
        id,
        horizon_client::crash::native_mod_call().and_then(|call| call.id),
    ) {
        return Err("post shader id must belong to the calling mod".into());
    }
    Ok(())
}

impl Game<'_> {
    pub fn native_register_post_shader(
        &mut self,
        id: &str,
        wgsl: &str,
        uniforms_layout: NativePostUniformLayout,
    ) -> Result<(), String> {
        owned_shader(id)?;
        let renderer = self.renderer.as_mut().ok_or("renderer unavailable")?;
        self.native_post_shaders.register(
            &mut renderer.native_services.post,
            &renderer.device,
            renderer.config.format.remove_srgb_suffix(),
            id,
            wgsl,
            uniforms_layout,
        )
    }

    pub fn native_set_post_shader_uniforms(
        &mut self,
        id: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        owned_shader(id)?;
        let renderer = self.renderer.as_mut().ok_or("renderer unavailable")?;
        self.native_post_shaders.set_uniforms(
            &mut renderer.native_services.post,
            &renderer.queue,
            id,
            bytes,
        )
    }

    pub fn native_enable_post_shader(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        owned_shader(id)?;
        let renderer = self.renderer.as_mut().ok_or("renderer unavailable")?;
        self.native_post_shaders
            .enable(&mut renderer.native_services.post, id, enabled)
    }

    pub fn native_unregister_post_shader(&mut self, id: &str) -> Result<(), String> {
        owned_shader(id)?;
        let renderer = self.renderer.as_mut().ok_or("renderer unavailable")?;
        self.native_post_shaders
            .unregister(&mut renderer.native_services.post, id)
    }

    pub fn native_post_shader_state(&self, id: &str) -> Result<bool, String> {
        owned_shader(id)?;
        let renderer = self.renderer.as_ref().ok_or("renderer unavailable")?;
        self.native_post_shaders
            .state(&renderer.native_services.post, id)
    }

    pub(super) fn restore_native_services(&mut self, renderer: &mut horizon_client::gfx::Renderer) {
        self.native_post_shaders.restore(
            &mut renderer.native_services.post,
            &renderer.device,
            &renderer.queue,
            renderer.config.format.remove_srgb_suffix(),
        );
        renderer
            .native_services
            .draws
            .restore_failures(&self.native_world_lines.failures);
        for (owner, texture) in self.native_world_lines.textures.values_mut() {
            match renderer.hud.native_world_texture_restore(
                &renderer.device,
                &renderer.queue,
                texture,
            ) {
                Ok(restored) => *texture = restored,
                Err(error) => renderer
                    .native_services
                    .draws
                    .disable(owner, &error.to_string()),
            }
        }
    }
}
