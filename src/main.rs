use bytes::BytesMut;

use clap::Clap;

use failure::{Error, format_err};

use prost::Message;

use std::io::{BufReader, Write};
use std::fs::{create_dir_all, File, OpenOptions};
use std::path::{Path, PathBuf};

mod parser;
use parser::{MarketXmlConfig, MarketXmlParser, ParsedItem};

pub(crate) mod market_xml {
    include!(concat!(env!("OUT_DIR"), "/market_xml.rs"));
}

#[derive(Clap, Debug)]
struct Opts {
    #[clap(long = "offers-chunk", default_value = "50000")]
    offers_chunk_size: u32,
    #[clap(long = "output-dir", short = "o")]
    output_dir: PathBuf,
    xml_file: PathBuf,
}

fn main() -> Result<(), Error> {
    let opts = Opts::parse();
    if opts.offers_chunk_size == 0 {
        return Err(format_err!("offers-chunk must be greater than 0"));
    }
    println!("opts: {:?}", opts);

    let file_reader = BufReader::new(File::open(opts.xml_file.as_path())?);
    let mut parser = MarketXmlParser::new(MarketXmlConfig::default(), file_reader);

    if !opts.output_dir.exists() {
        create_dir_all(&opts.output_dir)?;
    }

    let mut buf = BytesMut::new();
    let mut offers = market_xml::Offers::default();
    let mut chunk_ix = 0;
    let mut total_offers = 0;
    let mut offers_with_errors = 0;
    for item_res in parser {
        match item_res {
            Ok(ParsedItem::Offer(order)) => {
                offers.offers.push(order);
                total_offers += 1;
            }
            Ok(ParsedItem::Shop(shop)) => {
                write_shop(&opts.output_dir, shop, &mut buf)?;
            }
            Err(_) => {
                total_offers += 1;
                offers_with_errors += 1;
            },
        }

        if offers.offers.len() as u32 >= opts.offers_chunk_size {
            write_offers_chunk(&opts.output_dir, chunk_ix, &mut offers, &mut buf)?;
            chunk_ix += 1;
        }

//        if total_offers % 1000 == 0 {
//            println!("{}", total_offers);
//        }
    }

    if !offers.offers.is_empty() {
        write_offers_chunk(&opts.output_dir, chunk_ix, &mut offers, &mut buf)?;
    }

    println!("Total offers: {}", total_offers);
    println!("Offers with errors: {}", offers_with_errors);

    Ok(())
}

fn write_shop(
    out_dir: &Path, shop: market_xml::Shop, buf: &mut BytesMut
) -> Result<PathBuf, Error> {
    let mut shop_file_path = out_dir.to_path_buf();
    shop_file_path.push("shop.protobuf");
    let mut shop_file = OpenOptions::new().create_new(true).write(true)
        .open(&shop_file_path)?;
    shop.encode(buf)?;
    shop_file.write_all(buf)?;
    buf.clear();

    Ok(shop_file_path)
}

fn write_offers_chunk(
    out_dir: &Path,
    chunk_ix: u32,
    offers: &mut market_xml::Offers,
    buf: &mut BytesMut,
) -> Result<PathBuf, Error> {
    let mut offers_file_path = out_dir.to_path_buf();
    offers_file_path.push(format!("offers-{}.protobuf", chunk_ix));
    let mut offers_file = OpenOptions::new().create_new(true).write(true)
        .open(&offers_file_path)?;
    offers.encode(buf)?;
    offers_file.write_all(buf)?;
    offers.clear();
    buf.clear();

    Ok(offers_file_path)
}