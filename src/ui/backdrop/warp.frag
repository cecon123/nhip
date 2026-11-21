
  
  uniform sampler2D u_texture;
  uniform float u_time;
  uniform float u_intensity;
  in vec2 v_texCoord;
out vec4 fragColor;

  vec3 mod289(vec3 x) { return x - floor(x * (1.0 / 289.0)) * 289.0; }
  vec2 mod289(vec2 x) { return x - floor(x * (1.0 / 289.0)) * 289.0; }
  vec3 permute(vec3 x) { return mod289(((x*34.0)+1.0)*x); }

  float snoise(vec2 v) {
    const vec4 C = vec4(0.211324865405187, 0.366025403784439,
                        -0.577350269189626, 0.024390243902439);
    vec2 i  = floor(v + dot(v, C.yy));
    vec2 x0 = v - i + dot(i, C.xx);
    vec2 i1 = (x0.x > x0.y) ? vec2(1.0, 0.0) : vec2(0.0, 1.0);
    vec4 x12 = x0.xyxy + C.xxzz;
    x12.xy -= i1;
    i = mod289(i);
    vec3 p = permute(permute(i.y + vec3(0.0, i1.y, 1.0)) + i.x + vec3(0.0, i1.x, 1.0));
    vec3 m = max(0.5 - vec3(dot(x0,x0), dot(x12.xy,x12.xy), dot(x12.zw,x12.zw)), 0.0);
    m = m*m; m = m*m;
    vec3 x = 2.0 * fract(p * C.www) - 1.0;
    vec3 h = abs(x) - 0.5;
    vec3 ox = floor(x + 0.5);
    vec3 a0 = x - ox;
    m *= 1.79284291400159 - 0.85373472095314 * (a0*a0 + h*h);
    vec3 g;
    g.x = a0.x * x0.x + h.x * x0.y;
    g.yz = a0.yz * x12.xz + h.yz * x12.yw;
    return 130.0 * dot(m, g);
  }

  void main() {
    vec2 uv = v_texCoord;
    float t = u_time * 0.058;

    // Gentle orbital drift so cover color clouds breathe smoothly across the viewport (Spotify Kawarp feel).
    vec2 center = uv - 0.5;
    float ang = sin(t * 0.45) * 0.20 + cos(t * 0.28) * 0.14;
    float ca = cos(ang), sa = sin(ang);
    vec2 rotUV = mat2(ca, -sa, sa, ca) * center + 0.5;
    float centerWeight = 0.42 + 0.58 * (1.0 - smoothstep(0.0, 0.92, length(center)));

    // Smooth low-frequency 2-octave cloud flow (no high-frequency streaks or creases).
    float n1 = snoise(rotUV * 0.42 + vec2(t, t * 0.68));
    float n2 = snoise(rotUV * 0.42 + vec2(-t * 0.74, t * 0.52) + vec2(37.0, 61.0));
    vec2 flow1 = vec2(n1, n2);

    float n3 = snoise(rotUV * 0.78 + flow1 * 0.28 + vec2(t * 0.72, -t * 0.58) + vec2(93.0, 17.0));
    float n4 = snoise(rotUV * 0.78 + flow1 * 0.28 + vec2(-t * 0.62, t * 0.66) + vec2(19.0, 89.0));

    vec2 warp = (flow1 * 0.72 + vec2(n3, n4) * 0.28) * centerWeight;
    vec2 warpedUV = clamp(rotUV + warp * (u_intensity * 0.34), 0.04, 0.96);

    // 9-tap wide circular bokeh filter produces creamy, luminous clouds matching Spotify Desktop.
    vec2 sampleUV = vec2(warpedUV.x, 1.0 - warpedUV.y);
    float r1 = 0.048;
    float r2 = 0.088;
    vec4 acc = texture(u_texture, sampleUV) * 0.24;
    acc += texture(u_texture, clamp(sampleUV + vec2( r1, 0.0), 0.01, 0.99)) * 0.11;
    acc += texture(u_texture, clamp(sampleUV + vec2(-r1, 0.0), 0.01, 0.99)) * 0.11;
    acc += texture(u_texture, clamp(sampleUV + vec2(0.0,  r1), 0.01, 0.99)) * 0.11;
    acc += texture(u_texture, clamp(sampleUV + vec2(0.0, -r1), 0.01, 0.99)) * 0.11;
    acc += texture(u_texture, clamp(sampleUV + vec2( r2 * 0.707,  r2 * 0.707), 0.01, 0.99)) * 0.08;
    acc += texture(u_texture, clamp(sampleUV + vec2(-r2 * 0.707,  r2 * 0.707), 0.01, 0.99)) * 0.08;
    acc += texture(u_texture, clamp(sampleUV + vec2( r2 * 0.707, -r2 * 0.707), 0.01, 0.99)) * 0.08;
    acc += texture(u_texture, clamp(sampleUV + vec2(-r2 * 0.707, -r2 * 0.707), 0.01, 0.99)) * 0.08;
    fragColor = acc;
  }
