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
        renderer.native_services.post.register(
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
        renderer
            .native_services
            .post
            .set_uniforms(&renderer.queue, id, bytes)
    }

    pub fn native_enable_post_shader(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        owned_shader(id)?;
        self.renderer
            .as_mut()
            .ok_or("renderer unavailable")?
            .native_services
            .post
            .enable(id, enabled)
    }
}
