
  
  uniform sampler2D u_texture;
  uniform float u_alpha;
  uniform vec2 u_header_fade;
  uniform float u_radius;
  uniform float u_sidebar;
  uniform float u_saturation;
  uniform float u_dithering;
  uniform float u_time;
  uniform float u_scale;
  uniform vec2 u_resolution;
  uniform vec4 u_uv_rect;   // (u0, v0, uw, vh) inside the parent panel; (0,0,1,1) for full panel
  uniform float u_frost;    // 0.0 = normal backdrop, >0.0 = frosted glass backdrop blur (radius in UV)
  uniform float u_tint;     // white translucent glass tint strength (e.g. 0.075 for GlassCard)
  in vec2 v_texCoord;
out vec4 fragColor;

  highp float hash(highp vec3 p) {
    p = fract(p * 0.1031);
    p += dot(p, p.zyx + 31.32);
    return fract((p.x + p.y) * p.z);
  }

  vec3 sample_source(vec2 uv) {
    vec2 su = clamp((uv - 0.5) / u_scale + 0.5, 0.0, 1.0);
    if (u_frost <= 0.0001) {
      return texture(u_texture, su).rgb;
    }
    // 13-tap disc Gaussian backdrop blur for frosted GlassCard surfaces.
    float r1 = u_frost;
    float r2 = u_frost * 2.05;
    float d2 = r2 * 0.70710678;
    vec3 acc = texture(u_texture, su).rgb * 0.20;
    acc += texture(u_texture, clamp(su + vec2( r1, 0.0), 0.0, 1.0)).rgb * 0.10;
    acc += texture(u_texture, clamp(su + vec2(-r1, 0.0), 0.0, 1.0)).rgb * 0.10;
    acc += texture(u_texture, clamp(su + vec2(0.0,  r1), 0.0, 1.0)).rgb * 0.10;
    acc += texture(u_texture, clamp(su + vec2(0.0, -r1), 0.0, 1.0)).rgb * 0.10;
    acc += texture(u_texture, clamp(su + vec2( d2,  d2), 0.0, 1.0)).rgb * 0.05;
    acc += texture(u_texture, clamp(su + vec2(-d2,  d2), 0.0, 1.0)).rgb * 0.05;
    acc += texture(u_texture, clamp(su + vec2( d2, -d2), 0.0, 1.0)).rgb * 0.05;
    acc += texture(u_texture, clamp(su + vec2(-d2, -d2), 0.0, 1.0)).rgb * 0.05;
    acc += texture(u_texture, clamp(su + vec2( r2, 0.0), 0.0, 1.0)).rgb * 0.05;
    acc += texture(u_texture, clamp(su + vec2(-r2, 0.0), 0.0, 1.0)).rgb * 0.05;
    acc += texture(u_texture, clamp(su + vec2(0.0,  r2), 0.0, 1.0)).rgb * 0.05;
    acc += texture(u_texture, clamp(su + vec2(0.0, -r2), 0.0, 1.0)).rgb * 0.05;
    return acc;
  }

  void main() {
    vec2 panel_uv = clamp(u_uv_rect.xy + v_texCoord * u_uv_rect.zw, 0.0, 1.0);
    vec3 raw_rgb = sample_source(panel_uv);

    // Calibrated Kawarp grading matching Spicy Lyrics spicy-dynamic-bg.css
    // (`saturate(2.5) brightness(0.65)` page, `brightness(0.60) saturate(2.65)` sidebar).
    vec3 lin = pow(max(raw_rgb, vec3(0.0)), vec3(2.2));
    float luma = dot(lin, vec3(0.2126, 0.7152, 0.0722));
    float sat = mix(2.08, 2.28, u_sidebar) * (u_saturation / 1.5);
    vec3 graded = max(mix(vec3(luma), lin, sat), vec3(0.0));
    graded = pow(graded, vec3(1.08));
    graded = graded / (vec3(1.0) + max(graded - vec3(0.62), vec3(0.0)) * 0.62);
    vec3 srgb = pow(clamp(graded, 0.0, 1.0), vec3(1.0 / 2.2));

    vec2 center = panel_uv - 0.5;
    float t = u_time * 0.10;
    vec2 lightPos = vec2(sin(t * 0.55) * 0.28, cos(t * 0.42) * 0.24);
    float bloom = 1.0 + 0.14 * exp(-dot(center - lightPos, center - lightPos) * 1.9);
    float shadow = 1.0 - 0.18 * exp(-dot(center + lightPos, center + lightPos) * 1.7);
    float vignette = (1.0 - dot(center, center) * 0.46) * bloom * shadow;
    float brightness = mix(0.70, 0.63, u_sidebar);
    vec3 rgb = clamp(srgb * (brightness * vignette), 0.0, 1.0);

    if (u_frost > 0.0001) {
      // Frosted GlassCard surface (`NPVLyrics.css:15-16` / `default.css:387-388`):
      // gentle clarity lift + translucent white veil, never opaque gray.
      float fl = dot(rgb, vec3(0.2126, 0.7152, 0.0722));
      rgb = mix(vec3(fl), rgb, 1.12) * 1.04;
      rgb = mix(rgb, vec3(1.0), clamp(u_tint, 0.0, 0.25));
      rgb = clamp(rgb, 0.0, 1.0);
    }

    highp vec2 pixelPos = floor(v_texCoord * u_resolution);
    highp float noise = hash(vec3(pixelPos, floor(u_time * 60.0)));
    rgb = clamp(rgb + (noise - 0.5) * u_dithering, 0.0, 1.0);
    // egui_glow uses gamma-space textures and disables FRAMEBUFFER_SRGB.
    float alpha = u_alpha;
    vec2 q = abs((v_texCoord-0.5)*u_resolution) - (u_resolution*0.5-vec2(u_radius));
    float distance = length(max(q,0.0)) + min(max(q.x,q.y),0.0)-u_radius;
    alpha *= 1.0-smoothstep(-0.5,0.5,distance);
    if (u_header_fade.x >= 0.0) alpha *= 1.0 - smoothstep(u_header_fade.x, u_header_fade.y, 1.0-v_texCoord.y);
    fragColor = vec4(rgb * alpha, alpha);
  }
