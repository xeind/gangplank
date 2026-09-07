// CRT overlay: scanlines, aperture grille, vignette, flicker. Premultiplied
// alpha, meant to sit above the UI. Curvature and colour fringing need the
// screen as a texture (iChannel0), which this runtime does not provide yet.
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;

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

    // Dark overlay where lines are, plus a warm phosphor tint at low alpha.
    float dark = clamp(scan + grille + vig * 0.7 + flicker + roll, 0.0, 0.9);
    vec3 tint = vec3(0.35, 0.9, 0.5) * 0.05;
    fragColor = vec4(tint * (1.0 - dark), dark);
}
