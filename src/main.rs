use byteorder::{LittleEndian, ReadBytesExt};

use bytes::BytesMut;

use clap::Clap;

use flate2::bufread::GzDecoder;

use prost::{EncodeError, Message};

use snafu::{ResultExt, Snafu};

use std::io::{self, BufReader, Write, SeekFrom};
use std::io::prelude::*;
use std::ffi::OsStr;
use std::fs::{self, create_dir_all, File, OpenOptions};
use std::path::{Path, PathBuf};

mod parser;
use parser::{MarketXmlConfig, MarketXmlError, MarketXmlParser, ParsedItem};

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

#[derive(Debug, Snafu)]
enum CliError {
    #[snafu(display("Invalid option: {}", msg))]
    InvalidOpt { msg: String },
    #[snafu(display("Xml parse error: {}", msg))]
    ParseXml { msg: String },
    #[snafu(display("Cannot open an input file: {}", source))]
    OpenInputFile { source: io::Error },
    #[snafu(display("Cannot create an output directory: {}", source))]
    CreateOutputDir { source: io::Error },
    #[snafu(display("Cannot open an output file: {}", source))]
    OpenOutputFile { source: io::Error },
    #[snafu(display("Cannot write an output file: {}", source))]
    WriteOutputFile { source: io::Error },
    #[snafu(display("Error when encoding to protobuf: {}", source))]
    ProtobufEncode { source: EncodeError },
}

fn main() -> Result<(), CliError> {
    let opts = Opts::parse();
    if opts.offers_chunk_size == 0 {
        return Err(CliError::InvalidOpt { msg: "offers-chunk must be greater than 0".to_string() });
    }

    let (file_reader, file_size) = open_market_xml_file(opts.xml_file.as_path())
        .context(OpenInputFile)?;
    let mut parser = MarketXmlParser::new(MarketXmlConfig::default(), file_reader);

    if !opts.output_dir.exists() {
        create_dir_all(&opts.output_dir).context(CreateOutputDir)?;
    }

    let mut buf = BytesMut::new();
    let mut offers = market_xml::Offers::default();
    let mut errors = market_xml::Errors::default();
    let mut chunk_ix = 0;
    let mut total_offers = 0;
    let mut offers_with_errors = 0;
    while let Some(item_res) = parser.next_item() {
        match item_res {
            Ok(ParsedItem::Offer(order)) => {
                offers.offers.push(order);
                total_offers += 1;
            }
            Ok(ParsedItem::Shop(shop)) => {
                write_message(&opts.output_dir, "shop.protobuf", &shop, &mut buf)?;
            }
            Err(e) => {
                if let MarketXmlError::Xml {..} = e {
                    return Err(CliError::ParseXml { msg: format!("{}", e) });
                }
                errors.errors.push(market_xml::Error {
                    line: e.line() as u64,
                    message: format!("{}", e),
                    value: e.value().map(|v| v.to_string()).unwrap_or("".to_string()),
                });
                total_offers += 1;
                offers_with_errors += 1;
            },
        }

        if offers.offers.len() as u32 >= opts.offers_chunk_size {
            write_message(
                &opts.output_dir, &format!("offers-{}.protobuf", chunk_ix), &offers, &mut buf
            )?;
            offers.clear();
            chunk_ix += 1;
        }
    }

    if !offers.offers.is_empty() {
        write_message(
            &opts.output_dir, &format!("offers-{}.protobuf", chunk_ix), &offers, &mut buf
        )?;
    }

    if !errors.errors.is_empty() {
        write_message(&opts.output_dir, "errors.protobuf", &errors, &mut buf)?;
    }

    println!("Total offers: {}", total_offers);
    println!("Offers with errors: {}", offers_with_errors);

    Ok(())
}

fn open_market_xml_file(file_path: &Path) -> Result<(Box<dyn BufRead>, u64), io::Error> {
    let mut file = File::open(file_path)?;
    match file_path.extension() {
        Some(ext) if ext == OsStr::new("gz") => {
            let file_size = get_gzip_file_uncompressed_size(&mut file)? as u64;
            let reader = BufReader::new(GzDecoder::new(BufReader::new(file)));
            Ok((Box::new(reader), file_size))
        }
        _ => {
            let file_size = fs::metadata(file_path)?.len();
            let reader = BufReader::new(file);
            Ok((Box::new(reader), file_size))
        }
    }
}

fn get_gzip_file_uncompressed_size(file: &mut File) -> Result<u32, io::Error> {
    let orig_position = file.seek(SeekFrom::Current(0))?;
    file.seek(SeekFrom::End(-4))?;
    let size = file.read_u32::<LittleEndian>()?;
    file.seek(SeekFrom::Start(orig_position))?;
    return Ok(size);
}

fn write_message<M: Message>(
    out_dir: &Path, file_name: &str, msg: &M, buf: &mut BytesMut
) -> Result<PathBuf, CliError> {
    let mut file_path = out_dir.to_path_buf();
    file_path.push(file_name);
    let mut file = OpenOptions::new().create_new(true).write(true)
        .open(&file_path)
        .context(OpenOutputFile)?;
    msg.encode(buf).context(ProtobufEncode)?;
    file.write_all(buf).context(WriteOutputFile)?;
    buf.clear();

    Ok(file_path)
}
