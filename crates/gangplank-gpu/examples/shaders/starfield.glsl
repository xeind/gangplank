// Shadertoy style. Drop-in for Ghostty's custom-shader, minus iChannel0.
float hash(vec2 p) { return fract(sin(dot(p, vec2(41.3, 289.1))) * 45758.5453); }

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    vec3 col = vec3(0.01, 0.02, 0.05);
    for (float layer = 1.0; layer <= 4.0; layer += 1.0) {
        float z = fract(iTime * 0.08 * layer);
        vec2 p = uv * (1.0 + layer * 2.0 * (1.0 - z));
        vec2 cell = floor(p * 12.0);
        vec2 local = fract(p * 12.0) - 0.5;
        float star = hash(cell + layer);
        if (star > 0.92) {
            float d = length(local - (vec2(hash(cell * 1.7), hash(cell * 2.3)) - 0.5) * 0.8);
            float glow = exp(-d * d * 60.0) * (1.0 - z) * (star - 0.9) * 12.0;
            col += glow * mix(vec3(0.6, 0.8, 1.0), vec3(1.0, 0.8, 0.6), hash(cell));
        }
    }
    if (iMouse.z >= 0.0 && iMouse.x + iMouse.y > 0.0) {
        float d = length(fragCoord - iMouse.xy) / iResolution.y;
        col += vec3(0.4, 0.3, 0.8) * exp(-d * d * 40.0);
    }
    fragColor = vec4(col, 1.0);
}
