fn g() -> u8 {
    1
}
fn main() {
    let f: fn() -> u8 = g;
    let h: Box<dyn Fn() -> u8> = Box::new(|| 1);
    let b: bool = 1 > 2;
}
