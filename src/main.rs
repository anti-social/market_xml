use byteorder::{LittleEndian, ReadBytesExt};

use bytes::BytesMut;

use clap::Clap;

use flate2::bufread::GzDecoder;

use indicatif::{ProgressBar, ProgressStyle};

use prost::{EncodeError, Message};

use snafu::{ResultExt, Snafu};

use std::io::{self, BufReader, BufWriter, Write, SeekFrom};
use std::io::prelude::*;
use std::ffi::OsStr;
use std::fs::{self, create_dir_all, File, OpenOptions};
use std::path::{Path, PathBuf};

mod parser;
use parser::{MarketXmlConfig, MarketXmlError, MarketXmlParser, ParsedItem};

pub(crate) mod market_xml {
    include!(concat!(env!("OUT_DIR"), "/market_xml.rs"));
}

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Clap, Debug)]
struct Opts {
    #[clap(long = "offers-chunk", default_value = "50000")]
    offers_chunk_size: u32,
    #[clap(long = "output-dir", short = "o")]
    output_dir: PathBuf,
    #[clap(long = "no-progress")]
    no_progress: bool,
    #[clap(long = "dry-run")]
    dry_run: bool,
    #[clap(long = "verbose", short = "v")]
    verbose: bool,
    #[clap(long="if-modified-since")]
    if_modified_since: Option<String>,
    xml_file: String,
}

#[derive(Debug, Snafu)]
enum CliError {
    #[snafu(display("Invalid option: {}", msg))]
    InvalidOpt { msg: String },
    #[snafu(display("Xml parse error: {}", msg))]
    ParseXml { msg: String },
    #[snafu(display("Cannot open an input file {:?}: {}", path, source))]
    OpenInputFile { source: io::Error, path: PathBuf },
    #[snafu(display("Cannot create an output directory {:?}: {}", path, source))]
    CreateOutputDir { source: io::Error, path: PathBuf },
    #[snafu(display("Cannot open an output file {:?}: {}", path, source))]
    OpenOutputFile { source: io::Error, path: PathBuf },
    #[snafu(display("Cannot write an output file {:?}: {}", path, source))]
    WriteOutputFile { source: io::Error, path: PathBuf },
    #[snafu(display("Error when encoding to protobuf: {}", source))]
    ProtobufEncode { source: EncodeError },
    #[snafu(display("Error when downloading an xml file: {}", source))]
    Reqwest { source: reqwest::Error },
}

fn main() -> Result<(), CliError> {
    env_logger::init();

    let opts = Opts::parse();
    if opts.offers_chunk_size == 0 {
        return Err(CliError::InvalidOpt { msg: "offers-chunk must be greater than 0".to_string() });
    }

    let (file_reader, file_size) = if opts.xml_file.starts_with("http://") || opts.xml_file.starts_with("https://") {
        let client = reqwest::blocking::ClientBuilder::new()
            .gzip(true)
            .build()
            .context(ReqwestSnafu)?;
        let request = client.get(&opts.xml_file);
        let request = if let Some(if_modified_since) = opts.if_modified_since {
            request.header(reqwest::header::IF_MODIFIED_SINCE, if_modified_since)
        } else {
            request
        };
        let response = request
            .send()
            .context(ReqwestSnafu)?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            println!("Not modified");
            return Ok(());
        }
        if let Some(Ok(last_modified)) = response.headers().get(reqwest::header::LAST_MODIFIED).map(|v| v.to_str()) {
            if !opts.dry_run {
                ensure_output_dir(&opts.output_dir)?;
            }
            let last_modified_path = opts.output_dir.join("last-modified.txt");
            std::fs::write(&last_modified_path, last_modified)
                .context(WriteOutputFileSnafu { path: last_modified_path })?;
        }
        let content_length = response.content_length();
        (Box::new(BufReader::new(response)) as Box<dyn BufRead>, content_length)
    } else {
        open_market_xml_file(PathBuf::from(&opts.xml_file).as_path())
            .context(OpenInputFileSnafu { path: opts.xml_file })?
    };
    let mut parser = MarketXmlParser::new(MarketXmlConfig::default(), file_reader);

    if !opts.dry_run {
        ensure_output_dir(&opts.output_dir)?;
    }

    let progressbar = match (opts.no_progress, file_size) {
        (false, Some(file_size)) => {
        let pb = ProgressBar::new(file_size);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("[{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta}) parsing file")
                    .progress_chars("#>-")
            );
            Some(pb)
        }
        _ => None
    };

    let mut buf = BytesMut::new();
    let mut errors = market_xml::Errors::default();
    let mut available_offer_ids = market_xml::OfferIds::default();
    let mut unavailable_offer_ids = market_xml::OfferIds::default();
    let mut availability_missing_offer_ids = market_xml::OfferIds::default();
    let mut chunk_ix = 0;
    let mut chunk_offers = 0;
    let mut offers_writer = if !opts.dry_run {
        Some(
            DelimitedMessageWriter::open(
                &opts.output_dir, &format!("offers-{}.protobuf-delimited", chunk_ix)
            )?
        )
    } else {
        None
    };
    let mut total_offers = 0;
    let mut offers_with_errors = 0;
    loop {
        match parser.next_item() {
            Ok(ParsedItem::Offer(offer)) => {
                match offer.available {
                    Some(true) => {
                        available_offer_ids.ids.push(offer.id.clone());
                    }
                    Some(false) => {
                        unavailable_offer_ids.ids.push(offer.id.clone());
                    }
                    None => {
                        availability_missing_offer_ids.ids.push(offer.id.clone());
                    }
                }
                if offer.available.unwrap_or(false) {

                }
                if let Some(ref mut offers_writer) = offers_writer {
                    offers_writer.write(&offer, &mut buf)?;
                    chunk_offers += 1;
                }
                if chunk_offers == opts.offers_chunk_size {
                    chunk_ix += 1;
                    chunk_offers = 0;
                    offers_writer = Some(
                        DelimitedMessageWriter::open(
                            &opts.output_dir, &format!("offers-{}.protobuf-delimited", chunk_ix)
                        )?
                    );
                }
                total_offers += 1;
            }
            Ok(ParsedItem::YmlCatalog(yml_catalog)) => {
                if !opts.dry_run {
                    write_message(&opts.output_dir, "yml_catalog.protobuf", &yml_catalog, &mut buf)?;
                }
            }
            Ok(ParsedItem::Eof) => {
                break;
            }
            Err(e) => {
                if let MarketXmlError::Xml {..} = e {
                    return Err(CliError::ParseXml { msg: format!("{}", e) });
                }
                if opts.verbose {
                    if let Some(err_value) = e.value() {
                        log::error!("Line {}: {}: {}", e.line(), e, err_value);
                    } else {
                        log::error!("Line {}: {}", e.line(), e);
                    }
                }
                errors.errors.push(market_xml::Error {
                    line: e.line() as u64,
                    column: e.column() as u64,
                    message: format!("{}", e),
                    value: e.value().map(|v| v.to_string()).unwrap_or("".to_string()),
                });
                total_offers += 1;
                offers_with_errors += 1;
            },
        }

        progressbar.as_ref().map(|pb| {
            let cur_pos = parser.buffer_position() as u64;
            if cur_pos - pb.position() > file_size.unwrap() / 100 {
                pb.set_position(cur_pos);
            }
        });
    }

    if !opts.dry_run {
        available_offer_ids.ids.sort_unstable();
        unavailable_offer_ids.ids.sort_unstable();
        availability_missing_offer_ids.ids.sort_unstable();
        write_message(
            &opts.output_dir,
            &format!("offer-ids-available.protobuf"),
            &available_offer_ids,
            &mut buf
        )?;
        write_message(
            &opts.output_dir,
            &format!("offer-ids-unavailable.protobuf"),
            &unavailable_offer_ids,
            &mut buf
        )?;
        write_message(
            &opts.output_dir,
            &format!("offer-ids-availability-missing.protobuf"),
            &availability_missing_offer_ids,
            &mut buf
        )?;
    }

    if !errors.errors.is_empty() && !opts.dry_run {
        write_message(
            &opts.output_dir, "errors.protobuf", &errors, &mut buf
        )?;
    }

    progressbar.map(|pb| pb.finish());

    println!("Total offers: {total_offers}");
    println!("Offers with errors: {offers_with_errors}");

    Ok(())
}

fn ensure_output_dir(output_dir: &Path) -> Result<(), CliError> {
    if !output_dir.exists() {
        create_dir_all(&output_dir)
            .context(CreateOutputDirSnafu { path: output_dir.clone() })?;
    }
    Ok(())
}

fn open_market_xml_file(file_path: &Path) -> Result<(Box<dyn BufRead>, Option<u64>), io::Error> {
    let mut file = File::open(file_path)?;
    match file_path.extension() {
        Some(ext) if ext == OsStr::new("gz") => {
            let file_size = get_gzip_file_uncompressed_size(&mut file)? as u64;
            let reader = BufReader::new(GzDecoder::new(BufReader::new(file)));
            Ok((Box::new(reader), Some(file_size)))
        }
        _ => {
            let file_size = fs::metadata(file_path)?.len();
            let reader = BufReader::new(file);
            Ok((Box::new(reader), Some(file_size)))
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
        .context(OpenOutputFileSnafu { path: file_path.clone() })?;
    msg.encode(buf).context(ProtobufEncodeSnafu)?;
    file.write_all(buf)
        .context(WriteOutputFileSnafu { path: file_path.clone() })?;
    buf.clear();

    Ok(file_path)
}

struct DelimitedMessageWriter {
    file_path: PathBuf,
    writer: BufWriter<File>,
}

impl DelimitedMessageWriter {
    fn open(out_dir: &Path, file_name: &str) -> Result<Self, CliError> {
        let mut file_path = out_dir.to_path_buf();
        file_path.push(file_name);
        let file = OpenOptions::new().create_new(true).write(true)
            .open(&file_path)
            .context(OpenOutputFileSnafu { path: file_path.clone() })?;
        Ok(Self {
            file_path,
            writer: BufWriter::new(file),
        })
    }

    fn write<M: Message>(&mut self, msg: &M, buf: &mut BytesMut) -> Result<(), CliError> {
        msg.encode_length_delimited(buf).context(ProtobufEncodeSnafu)?;
        self.writer.write_all(buf)
            .context(WriteOutputFileSnafu { path: self.file_path.clone() })?;
        buf.clear();
    
        Ok(())
    }
}
