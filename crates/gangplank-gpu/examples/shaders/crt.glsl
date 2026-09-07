// CRT overlay: barrel curvature, colour fringing, scanlines, aperture
// grille, vignette, flicker. Reads the UI under it through iChannel0, so the
// element needs `.backdrop(margin)`. Opaque: everything under it is redrawn
// through the tube.
vec2 curve(vec2 uv) {
    uv = uv * 2.0 - 1.0;
    vec2 offset = abs(uv.yx) / vec2(6.0, 4.5);
    uv = uv + uv * offset * offset;
    return uv * 0.5 + 0.5;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = curve(fragCoord / iResolution.xy);

    // Outside the curved tube is the bezel.
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        fragColor = vec4(0.0, 0.0, 0.0, 1.0);
        return;
    }

    // Fringe: shift red and blue two device pixels each way, inside the
    // 4 px backdrop margin the demo asks for.
    float fringe = 2.0 / iResolution.x;
    vec3 col;
    col.r = texture(iChannel0, uv + vec2(fringe, 0.0)).r;
    col.g = texture(iChannel0, uv).g;
    col.b = texture(iChannel0, uv - vec2(fringe, 0.0)).b;

    // Scanlines: one dark line every 3 device pixels, scrolling slowly.
    float line = 0.5 + 0.5 * sin((fragCoord.y + iTime * 12.0) * 6.28318 / 3.0);
    float scan = 0.28 * line;

    // Aperture grille: faint vertical RGB stripes.
    float grille = 0.06 * (0.5 + 0.5 * sin(fragCoord.x * 6.28318 / 3.0));

    // Vignette: darken toward the corners.
    vec2 c = uv - 0.5;
    float vig = smoothstep(0.35, 0.75, dot(c, c) * 2.2);

    // Flicker: a slow 60 Hz-ish shimmer plus a rare roll bar.
    float flicker = 0.03 * sin(iTime * 120.0);
    float roll = 0.12 * smoothstep(0.0, 0.04, abs(fract(uv.y - iTime * 0.08) - 0.5) - 0.46);

    float dark = clamp(scan + grille + vig * 0.7 + flicker + roll, 0.0, 0.9);
    vec3 tint = vec3(0.35, 0.9, 0.5) * 0.05;
    fragColor = vec4(col * (1.0 - dark) + tint, 1.0);
}
