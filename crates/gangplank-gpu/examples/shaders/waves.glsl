// Shadertoy style: layered sine waves with a palette.
vec3 palette(float t) {
    return 0.5 + 0.5 * cos(6.28318 * (vec3(1.0, 1.0, 1.0) * t + vec3(0.0, 0.33, 0.67)));
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    float y = uv.y;
    float acc = 0.0;
    for (int i = 0; i < 6; i++) {
        float fi = float(i);
        float w = sin(uv.x * (3.0 + fi) + iTime * (0.6 + fi * 0.2)) * 0.05;
        acc += smoothstep(0.02, 0.0, abs(y - (0.2 + fi * 0.12) - w));
    }
    vec3 col = mix(vec3(0.05, 0.03, 0.1), palette(uv.x * 0.5 + iTime * 0.1), acc);
    fragColor = vec4(col, 1.0);
}
