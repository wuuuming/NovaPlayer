// The backdrop's image effect for the GPU backends (now_playing.rs `FIELD`, whose `field_cpu` is
// the same program on the CPU): compiled when NovaPlayer is built (build.rs, novaui-shaders).
fn np_field(v: VOut) -> vec4<f32> {
    let rect = v.p0;
    let p = floor(v.pos.xy) + vec2<f32>(0.5, 0.5);
    let n = vec2<f32>(textureDimensions(tex_a, 0)) - vec2<f32>(1.0, 1.0);
    let gp = (p - rect.xy) / rect.zw * n;
    // Inner lattice points wander (±0.34 cell); the rim stays put.
    let rim = clamp(min(min(gp.x, n.x - gp.x), min(gp.y, n.y - gp.y)), 0.0, 1.0);
    let drift = v.c0.x;
    let d = vec2<f32>(
        0.34 * sin(drift * 0.113 + gp.y * 1.3 + gp.x * 0.7),
        0.34 * cos(drift * 0.097 + gp.y * 0.9 + gp.x * 1.9)) * rim;
    let s = clamp(gp - d, vec2<f32>(0.0, 0.0), n);
    let c = bilinear(tex_a, s + vec2<f32>(0.5, 0.5));
    let cell = rect.z / n.x;
    let shade_from = rect.x + v.c0.y;
    let keep = 1.0 - v.c0.z * clamp((p.x - (shade_from - cell)) / max(cell, 1.0), 0.0, 1.0);
    return vec4<f32>(c.rgb * keep, c.a);
}
