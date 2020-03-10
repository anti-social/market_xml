mod parser;

pub(crate) mod market_xml {
    include!(concat!(env!("OUT_DIR"), "/market_xml.rs"));
}

fn main() {
    let offer = market_xml::Offer::default();

    println!("Offer: {:?}", offer);
}
