fn main() {
    let a = (1, 1);
    let _ = [a];
    {
        let _ = a;
    };
}
