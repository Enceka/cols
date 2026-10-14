//! Vector operations shared by collision and movement.
pub(crate) type V3 = [f32; 3];
pub(crate) fn add(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] + b[i])
}
pub(crate) fn sub(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] - b[i])
}
pub(crate) fn mul(a: V3, s: f32) -> V3 {
    a.map(|x| x * s)
}
pub(crate) fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn length(a: V3) -> f32 {
    dot(a, a).sqrt()
}
pub(crate) fn normalized(a: V3) -> V3 {
    let n = length(a);
    if n > 1e-8 { mul(a, 1.0 / n) } else { [0.0; 3] }
}
