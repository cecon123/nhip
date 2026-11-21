
  
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
  in vec2 v_texCoord;
out vec4 fragColor;

  highp float hash(highp vec3 p) {
    p = fract(p * 0.1031);
    p += dot(p, p.zyx + 31.32);
    return fract((p.x + p.y) * p.z);
  }

  void main() {
    vec2 uv = (v_texCoord - 0.5) / u_scale + 0.5;
    uv = clamp(uv, 0.0, 1.0);

    vec4 color = texture(u_texture, uv);

    // Creamy luminous grading matching Spotify Kawarp dynamic backdrop (broad soft glow, clean edge vignette).
    vec3 lin = pow(max(color.rgb, vec3(0.0)), vec3(2.2));
    float luma = dot(lin, vec3(0.2126, 0.7152, 0.0722));
    float sat = mix(2.02, 2.24, u_sidebar) * (u_saturation / 1.5);
    vec3 graded = max(mix(vec3(luma), lin, sat), vec3(0.0));
    graded = pow(graded, vec3(1.06));
    graded = graded / (vec3(1.0) + max(graded - vec3(0.72), vec3(0.0)) * 0.50);
    vec3 srgb = pow(clamp(graded, 0.0, 1.0), vec3(1.0 / 2.2));

    vec2 center = v_texCoord - 0.5;
    float t = u_time * 0.10;
    vec2 lightPos = vec2(sin(t * 0.55) * 0.28, cos(t * 0.42) * 0.24);
    float bloom = 1.0 + 0.16 * exp(-dot(center - lightPos, center - lightPos) * 1.9);
    float shadow = 1.0 - 0.16 * exp(-dot(center + lightPos, center + lightPos) * 1.7);
    float vignette = (1.0 - dot(center, center) * 0.44) * bloom * shadow;
    float brightness = mix(0.84, 0.74, u_sidebar);
    color.rgb = clamp(srgb * (brightness * vignette), 0.0, 1.0);

    highp vec2 pixelPos = floor(v_texCoord * u_resolution);
    highp float noise = hash(vec3(pixelPos, floor(u_time * 60.0)));
    color.rgb = clamp(color.rgb + (noise - 0.5) * u_dithering, 0.0, 1.0);
    // egui_glow uses gamma-space textures and disables FRAMEBUFFER_SRGB.
    float alpha = u_alpha;
    vec2 q = abs((v_texCoord-0.5)*u_resolution) - (u_resolution*0.5-vec2(u_radius));
    float distance = length(max(q,0.0)) + min(max(q.x,q.y),0.0)-u_radius;
    alpha *= 1.0-smoothstep(-0.5,0.5,distance);
    if (u_header_fade.x >= 0.0) alpha *= 1.0 - smoothstep(u_header_fade.x, u_header_fade.y, 1.0-v_texCoord.y);
    fragColor = vec4(color.rgb * alpha, alpha);
  }
