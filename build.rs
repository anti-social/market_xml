fn main() {
    prost_build::compile_protos(
        &[
            "src/market_xml/market_xml.proto",
        ],
        &["src/"]
    ).unwrap();
}
