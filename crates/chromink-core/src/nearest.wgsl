@group(0) @binding(0) var<storage, read> pixels: array<u32>;
@group(0) @binding(1) var<storage, read> palette: array<u32>;
@group(0) @binding(2) var<storage, read_write> indices: array<u32>;
@group(0) @binding(3) var<uniform> counts: vec4<u32>;
fn rgba(p: u32) -> vec4<i32> {
    return vec4<i32>(i32(p & 255u), i32((p >> 8u) & 255u), i32((p >> 16u) & 255u), i32(p >> 24u));
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if (index >= counts.x) { return; }
    let pixel = pixels[index];
    if ((pixel >> 24u) == 0u) { indices[index] = 0xffffffffu; return; }
    let color = rgba(pixel);
    var best = 0u;
    var minimum = 0xffffffffu;
    for (var i = 0u; i < counts.y; i = i + 1u) {
        let d = color - rgba(palette[i]);
        let squared = d * d;
        let distance = u32(squared.x + squared.y + squared.z + squared.w);
        // Strict comparison preserves the CPU's first-index tie rule.
        if (distance < minimum) { minimum = distance; best = i; }
    }
    indices[index] = best;
}
