fn main() {
    println!("var: {:?}", std::env::var("CARGO_CFG_DEBUG_ASSERTIONS"));
}
