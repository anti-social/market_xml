use quick_xml::{PositionWithLine, Reader as XmlReader, Error as XmlError};
use quick_xml::events::{Event, BytesStart};
use quick_xml::events::attributes::Attributes;

use snafu::{ResultExt, Snafu};

use std::collections::HashSet;
use std::io::prelude::BufRead;
use std::fmt::Display;
use std::str::{self, FromStr};

use crate::market_xml::{
    Category, Condition, Currency, DeliveryOption, Offer, Param, Price, Shop,
    YmlCatalog,
};

#[derive(Debug, Snafu)]
pub(crate) enum MarketXmlError {
    #[snafu(display("Xml error: {}", source))]
    Xml {
        source: XmlError,
        line: usize,
        column: usize,
    },
    #[snafu(display("Unexpected tag"))]
    UnexpectedTag {
        tag: String,
        line: usize,
        column: usize,
    },
    #[snafu(display("{}", msg))]
    InvalidUtf8 {
        msg: String,
        line: usize,
        column: usize,
        value: String,
    },
    #[snafu(display("{}", msg))]
    Validation {
        msg: String,
        line: usize,
        column: usize,
        value: String,
    }
}

impl MarketXmlError {
    pub(crate) fn line(&self) -> usize {
        use MarketXmlError::*;

        match *self {
            Xml { line, .. } => line,
            UnexpectedTag { line, .. } => line,
            InvalidUtf8 { line, .. } => line,
            Validation { line, .. } => line,
        }
    }

    pub(crate) fn column(&self) -> usize {
        use MarketXmlError::*;

        match *self {
            Xml { column, .. } => column,
            UnexpectedTag { column, .. } => column,
            InvalidUtf8 { column, .. } => column,
            Validation { column, .. } => column,
        }
    }

    pub(crate) fn value(&self) -> Option<&str> {
        use MarketXmlError::*;

        match self {
            Xml { .. } => None,
            UnexpectedTag { tag, .. } => Some(tag),
            InvalidUtf8 { .. } => None,
            Validation { value, .. } => Some(value)
        }
    }
}

pub(crate) struct MarketXmlConfig {
    offer_tags: HashSet<Vec<u8>>,
}

impl Default for MarketXmlConfig {
    fn default() -> Self {
        let mut offer_tags = HashSet::new();
        offer_tags.insert(b"offer".to_vec());
        Self {
            offer_tags,
        }
    }
}

pub(crate) struct MarketXmlParser<B: BufRead> {
    config: MarketXmlConfig,
    xml_reader: XmlReader<B, PositionWithLine>,
    buf: Vec<u8>,
    state: State,
    yml_catalog: YmlCatalog,
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum State {
    Begin,
    YmlCatalog,
    Shop,
    Offers,
    End,
}

#[derive(PartialEq, Debug)]
pub(crate) enum ParsedItem {
    Offer(Offer),
    YmlCatalog(YmlCatalog),
    Eof,
}

impl<B: BufRead> MarketXmlParser<B> {
    pub(crate) fn new(config: MarketXmlConfig, reader: B) -> Self {
        let mut xml_reader = XmlReader::from_reader_with_position_tracker(
            reader, PositionWithLine::default()
        );
        xml_reader.trim_text(true);
        Self {
            config,
            xml_reader,
            buf: vec!(),
            state: State::Begin,
            yml_catalog: YmlCatalog::default(),
        }
    }

    fn cur_line(&self) -> usize {
        self.xml_reader.position().line()
    }

    fn cur_column(&self) -> usize {
        self.xml_reader.position().column()
    }

    pub(crate) fn buffer_position(&self) -> usize {
        self.xml_reader.buffer_position()
    }

    fn xml_err_ctx(&self) -> Xml<usize, usize> {
        Xml {
            line: self.cur_line(),
            column: self.cur_column(),
        }
    }

    fn next_event(&mut self) -> Result<Event, MarketXmlError> {
        let line = self.cur_line();
        let column = self.cur_column();
        let event_res = self.xml_reader.read_event(&mut self.buf);
        match event_res {
            Ok(event) => Ok(event),
            Err(error) => {
                Err(MarketXmlError::Xml {
                    source: error,
                    line,
                    column,
                })
            }
        }
    }

    pub(crate) fn next_item(&mut self) -> Result<ParsedItem, MarketXmlError> {
        loop {
            match self.state {
                State::Begin => {
                    self.state = self.begin()?;
                }
                State::YmlCatalog => {
                    self.state = self.parse_yml_catalog()?;
                    if self.state == State::End {
                        return Ok(ParsedItem::YmlCatalog(self.yml_catalog.clone()));
                    }
                }
                State::Shop => {
                    self.state = self.parse_shop()?;
                }
                State::Offers => {
                    match self.parse_offers()? {
                        Some(offer) => {
                            return Ok(ParsedItem::Offer(offer));
                        }
                        None => {
                            self.state = State::Shop;
                        }
                    }
                }
                State::End => {
                    return Ok(ParsedItem::Eof);
                }
            }
        }
    }

    fn begin(&mut self) -> Result<State, MarketXmlError> {
        loop {
            match self.next_event()? {
                Event::Start(tag) => {
                    if tag.name() == b"yml_catalog" {
                        let tag = tag.to_owned();
                        self.parse_yml_catalog_attrs(&mut tag.attributes())?;
                        return Ok(State::YmlCatalog);
                    }
                    return Err(MarketXmlError::UnexpectedTag {
                        tag: String::from_utf8_lossy(tag.name()).to_string(),
                        line: self.cur_line(),
                        column: self.cur_column(),
                    });
                }
                Event::Eof => {
                    return Err(XmlError::UnexpectedEof("yandex market file".to_string()))
                        .context(self.xml_err_ctx());
                }
                _ => {}
            }
        }
    }

    fn parse_yml_catalog(&mut self) -> Result<State, MarketXmlError> {
        loop {
            match self.next_event()? {
                Event::Start(tag) => {
                    if tag.name() == b"shop" {
                        return Ok(State::Shop);
                    }
                }
                Event::End(tag) => {
                    if tag.name() == b"yml_catalog" {
                        return Ok(State::End);
                    }
                }
                Event::Eof => {
                    return Err(XmlError::UnexpectedEof("yml_catalog".to_string()))
                        .context(self.xml_err_ctx());
                }
                _ => {}
            }
        }
    }

    fn parse_yml_catalog_attrs(&mut self, attrs: &mut Attributes) -> Result<(), MarketXmlError> {
        for attr_res in attrs {
            let attr = attr_res.context(self.xml_err_ctx())?;
            match attr.key {
                b"date" => {
                    self.yml_catalog.date = self.decode_value(&attr.value)?.to_string();
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn parse_shop(&mut self) -> Result<State, MarketXmlError> {
        loop {
            match self.next_event()? {
                Event::Start(tag) |
                Event::Empty(tag) => {
                    if tag.name() == b"offers" {
                        return Ok(State::Offers);
                    }
                    let tag = tag.to_owned();
                    self.parse_shop_field(tag)?;
                }
                Event::End(tag) => {
                    if tag.name() == b"shop" {
                        return Ok(State::YmlCatalog);
                    }
                }
                Event::Eof => {
                    return Err(XmlError::UnexpectedEof("shop".to_string()))
                        .context(self.xml_err_ctx());
                },
                _ => {}
            }
        }
    }

    fn parse_shop_field(&mut self, tag: BytesStart) -> Result<(), MarketXmlError> {
        fn get_shop(yml_catalog: &mut YmlCatalog) -> &mut Shop {
            yml_catalog.shop.get_or_insert(Shop::default())
        }
        match tag.name() {
            b"name" => {
                get_shop(&mut self.yml_catalog).name = self.read_text()?;
            }
            b"company" => {
                get_shop(&mut self.yml_catalog).company = self.read_text()?;
            }
            b"url" => {
                get_shop(&mut self.yml_catalog).url = self.read_text()?;
            }
            b"currencies" => {
                get_shop(&mut self.yml_catalog).currencies = self.parse_currencies()?;
            }
            b"categories" => {
                get_shop(&mut self.yml_catalog).categories = self.parse_categories()?;
            }
            b"delivery-options" => {
                get_shop(&mut self.yml_catalog).delivery_options = self.parse_delivery_options()?;
            }
            _ => {}
        }
        Ok(())
    }

    fn parse_currencies(&mut self) -> Result<Vec<Currency>, MarketXmlError> {
        let mut currencies = vec!();
        loop {
            match self.next_event()? {
                Event::Start(tag) |
                Event::Empty(tag) => {
                    if tag.name() == b"currency" {
                        let tag = tag.into_owned();
                        currencies.push(self.parse_currency(&mut tag.attributes())?);
                    }
                }
                Event::End(tag) => {
                    if tag.name() == b"currencies" {
                        return Ok(currencies);
                    }
                }
                Event::Eof => {
                    return Err(XmlError::UnexpectedEof("currencies".to_string()))
                        .context(self.xml_err_ctx())?
                },
                _ => {}
            }
        }
    }

    fn parse_currency(&mut self, attrs: &mut Attributes) -> Result<Currency, MarketXmlError> {
        let mut currency = Currency::default();
        for attr_res in attrs {
            let attr = attr_res.context(self.xml_err_ctx())?;
            let value = self.decode_value(&attr.value)?.to_string();
            match attr.key {
                b"id" => {
                    currency.id = value;
                }
                b"rate" => {
                    currency.rate = value;
                }
                b"plus" => {
                    currency.plus = value;
                }
                _ => {}
            }
        }
        Ok(currency)
    }

    fn parse_categories(&mut self) -> Result<Vec<Category>, MarketXmlError> {
        let mut categories = vec!();
        loop {
            match self.next_event()? {
                Event::Start(tag) |
                Event::Empty(tag) => {
                    if tag.name() == b"category" {
                        let tag = tag.into_owned();
                        categories.push(self.parse_category(&mut tag.attributes())?);
                    }
                }
                Event::End(tag) => {
                    if tag.name() == b"categories" {
                        return Ok(categories);
                    }
                }
                Event::Eof => {
                    return Err(XmlError::UnexpectedEof("categories".to_string()))
                        .context(self.xml_err_ctx());
                },
                _ => {}
            }
        }
    }

    fn parse_category(&mut self, attrs: &mut Attributes) -> Result<Category, MarketXmlError> {
        let mut category = Category::default();
        for attr_res in attrs {
            let attr = attr_res.context(self.xml_err_ctx())?;
            match attr.key {
                b"id" => {
                    category.id = self.parse_value(&attr.value)?;
                }
                b"parentId" => {
                    category.parent_id = self.parse_value(&attr.value)?;
                }
                _ => {}
            }
        }
        category.name = self.read_text()?;
        Ok(category)
    }

    fn parse_offers(&mut self) -> Result<Option<Offer>, MarketXmlError> {
        loop {
            match self.next_event()? {
                Event::Start(tag) => {
                    if tag.name() == b"offer" {
                        let tag = tag.to_owned();
                        return Ok(Some(self.parse_offer(&mut tag.attributes())?));
                    }
                }
                Event::End(tag) => {
                    if tag.name() == b"offers" {
                        return Ok(None)
                    }
                }
                Event::Eof => {
                    return Err(XmlError::UnexpectedEof("offers".to_string()))
                        .context(self.xml_err_ctx());
                },
                _ => {}
            }

            self.buf.clear();
        }
    }

    fn parse_offer(&mut self, attrs: &mut Attributes) -> Result<Offer, MarketXmlError> {
        let mut offer = Offer::default();
        self.parse_offer_attributes(attrs, &mut offer)?;
        self.parse_offer_fields(&mut offer)?;
        Ok(offer)
    }

    fn parse_offer_attributes(
        &self, attrs: &mut Attributes, offer: &mut Offer
    ) -> Result<(), MarketXmlError> {
        for attr_res in attrs {
            let attr = attr_res.context(self.xml_err_ctx())?;
            match attr.key {
                b"id" => {
                    offer.id = self.decode_value(&attr.value)?.to_string();
                }
                b"type" => {
                    offer.r#type = self.decode_value(&attr.value)?.to_string();
                }
                b"bid" => {
                    offer.bid = self.parse_value(&attr.value)?;
                }
                b"cbid" => {
                    offer.cbid = self.parse_value(&attr.value)?;
                }
                b"available" => {
                    offer.available = match attr.value.as_ref() {
                        b"false" | b"0" => Some(false),
                        b"true" | b"1" => Some(true),
                        b"" => None,
                        _ => return Err(MarketXmlError::Validation {
                            msg: "parse bool".to_string(),
                            line: self.cur_line(),
                            column: self.cur_column(),
                            value: self.decode_value(&attr.value)?.to_string(),
                        }),
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn parse_offer_fields(&mut self, offer: &mut Offer) -> Result<(), MarketXmlError> {
        loop {
            let event = self.next_event()?;
            match event {
                Event::Start(tag) |
                Event::Empty(tag) => {
                    let tag = tag.into_owned();
                    self.parse_offer_field(tag, offer)?;
                }
                Event::End(tag) => {
                    if tag.name() == b"offer" {
                        break;
                    }
                }
                Event::Eof => {
                    return Err(XmlError::UnexpectedEof("Offer".to_string()))
                        .context(self.xml_err_ctx());
                },
                _ => {}
            }
        }
        Ok(())
    }

    fn parse_offer_field(&mut self, tag: BytesStart, offer: &mut Offer) -> Result<(), MarketXmlError> {
        match tag.name() {
            b"name" => {
                offer.name = self.read_text()?;
            }
            b"vendor" => {
                offer.vendor = self.read_text()?;
            }
            b"vendorCode" => {
                offer.vendor_code = self.read_text()?;
            }
            b"url" => {
                offer.url = self.read_text()?;
            }
            b"picture" => {
                offer.picture = self.read_text()?;
            }
            b"price" => {
                let tag = tag.to_owned();
                offer.price = Some(self.parse_price(&mut tag.attributes())?);
            }
            b"oldprice" => {
                let tag = tag.to_owned();
                offer.old_price = Some(self.parse_price(&mut tag.attributes())?);
            }
            b"currencyId" => {
                offer.currency_id = self.read_text()?;
            }
            b"categoryId" => {
                offer.category_id = self.read_value()?;
            }
            b"description" => {
                offer.description = self.read_text()?;
            }
            b"sales_notes" => {
                offer.sales_notes = self.read_text()?;
            }
            b"delivery" => {
                offer.delivery = self.read_opt()?;
            }
            b"pickup" => {
                offer.pickup = self.read_opt()?;
            }
            b"store" => {
                offer.store = self.read_opt()?;
            }
            b"downloadable" => {
                offer.downloadable = self.read_value()?;
            }
            b"enable_auto_discounts" => {
                offer.enable_auto_discounts = self.read_value()?;
            }
            b"min_quantity" => {
                offer.min_quantity = self.read_opt()?;
            }
            b"manufacturer_warranty" => {
                offer.manufacturer_warranty = self.read_value()?;
            }
            b"barcode" => {
                offer.barcodes.push(self.read_text()?);
            }
            b"param" => {
                let tag = tag.to_owned();
                offer.params.push(self.parse_param(&mut tag.attributes())?);
            }
            b"condition" => {
                let tag = tag.to_owned();
                offer.condition = Some(self.parse_condition(&mut tag.attributes())?);
            }
            b"credit-template" => {
                let tag = tag.to_owned();
                offer.credit_template_id = self.parse_credit_template(&mut tag.attributes())?
                    .unwrap_or("".to_string());
            }
            b"country_of_origin" => {
                offer.country_of_origin = self.read_text()?;
            }
            b"weight" => {
                offer.weight = self.read_value()?;
            }
            b"dimensions" => {
                offer.dimensions = self.read_text()?;
            }
            b"delivery-options" => {
                offer.delivery_options = self.parse_delivery_options()?;
            }
            b"pickup-options" => {
                offer.pickup_options = self.parse_delivery_options()?;
            }
            _ => {
                // TODO: save unknown fields into some dynamic message
                // println!("> {}", String::from_utf8_lossy(tag.name()));
            }
        }
        Ok(())
    }

    fn parse_delivery_options(&mut self) -> Result<Vec<DeliveryOption>, MarketXmlError> {
        let mut options = vec!();
        loop {
            match self.next_event()? {
                Event::Start(tag) |
                Event::Empty(tag) => {
                    if tag.name() == b"option" {
                        let tag = tag.into_owned();
                        options.push(self.parse_delivery_option(&mut tag.attributes())?);
                    }
                }
                Event::End(_) => {
                    break;
                }
                Event::Eof => {
                    return Err(XmlError::UnexpectedEof("Delivery options".to_string()))
                        .context(self.xml_err_ctx());
                }
                _ => {
                    return Err(XmlError::TextNotFound)
                        .context(self.xml_err_ctx())
                }
            }
        }
        Ok(options)
    }

    fn parse_delivery_option(&self, tag_attrs: &mut Attributes) -> Result<DeliveryOption, MarketXmlError> {
        let mut option = DeliveryOption::default();
        for attr_res in tag_attrs {
            let attr = attr_res.context(self.xml_err_ctx())?;
            match attr.key {
                b"cost" => {
                    option.cost = self.parse_value(&attr.value)?;
                }
                b"days" => {
                    option.days = self.decode_value(&attr.value)?.to_string();
                }
                b"order-before" => {
                    option.order_before = self.parse_opt(&attr.value)?;
                }
                _ => {}
            }
        }
        Ok(option)
    }

    fn parse_price(&mut self, tag_attrs: &mut Attributes) -> Result<Price, MarketXmlError> {
        let mut price = Price::default();
        price.price = self.read_value()?;
        for attr_res in tag_attrs {
            let attr = attr_res.context(self.xml_err_ctx())?;
            if attr.key == b"from" && attr.value.as_ref() == b"true" {
                price.from = true;
            }
        }
        Ok(price)
    }

    fn parse_param(&mut self, tag_attrs: &mut Attributes) -> Result<Param, MarketXmlError> {
        let mut param = Param::default();
        param.value = self.read_text()?;
        for attr_res in tag_attrs {
            let attr = attr_res.context(self.xml_err_ctx())?;
            let value = self.decode_value(&attr.value)?.to_string();
            match attr.key {
                b"name" => {
                    param.name = value;
                }
                b"unit" => {
                    param.unit = value;
                }
                b"id" => {
                    param.id = value;
                }
                b"valueid" => {
                    param.value_id = value;
                }
                _ => {}
            }
        }
        Ok(param)
    }

    fn parse_condition(&mut self, tag_attrs: &mut Attributes) -> Result<Condition, MarketXmlError> {
        let mut condition = Condition::default();
        for attr_res in tag_attrs {
            let attr = attr_res.context(self.xml_err_ctx())?;
            match attr.key {
                b"type" => {
                    condition.r#type = self.decode_value(&attr.value)?.to_string();
                }
                _ => {}
            }
        }
        loop {
            match self.next_event()? {
                Event::Start(ref tag) => {
                    let tag_name = tag.name();
                    match tag_name {
                        b"reason" => {
                            condition.reason = self.read_text()?;
                        }
                        _ => {}
                    }
                }
                Event::End(_) => break,
                Event::Eof => {
                    return Err(XmlError::UnexpectedEof("Condition".to_string()))
                        .context(self.xml_err_ctx());
                },
                _ => {
                    return Err(XmlError::TextNotFound)
                        .context(self.xml_err_ctx());
                },
            }
        }
        Ok(condition)
    }

    fn parse_credit_template(&self, tag_attrs: &mut Attributes) -> Result<Option<String>, MarketXmlError> {
        for attr_res in tag_attrs {
            let attr = attr_res.context(self.xml_err_ctx())?;
            if attr.key == b"id" {
                return Ok(Some(self.decode_value(&attr.value)?.to_string()));
            }
        }
        Ok(None)
    }

    fn decode_value<'a, 'b>(&'a self, v: &'b[u8]) -> Result<&'b str, MarketXmlError> {
        str::from_utf8(v)
            .map_err(|e| {
                MarketXmlError::InvalidUtf8 {
                    msg: format!("{}", e),
                    value: String::from_utf8_lossy(v).to_string(),
                    line: self.cur_line(),
                    column: self.cur_column(),
                }
            })
    }

    fn read_text(&mut self) -> Result<String, MarketXmlError> {
        self.read_text_and_parse(|s, _, _| Ok(s.to_string()))
    }

    fn read_value<T>(&mut self) -> Result<T, MarketXmlError>
    where
        T: FromStr,
        T::Err: Display,
    {
        self.read_text_and_parse(|s, line, column| {
            s.parse().map_err(|e| {
                MarketXmlError::Validation {
                    msg: format!("{}", e),
                    line,
                    column,
                    value: s.to_string(),
                }
            })
        })
    }

    fn read_opt<T>(&mut self) -> Result<Option<T>, MarketXmlError>
    where
        T: FromStr,
        T::Err: Display,
    {
        self.read_text_and_parse(|s, line, column| {
            if s == "" {
                Ok(None)
            } else {
                Some(
                    s.parse().map_err(|e| {
                        MarketXmlError::Validation {
                            msg: format!("{}", e),
                            line,
                            column,
                            value: s.to_string(),
                        }
                    })
                ).transpose()
            }
        })
    }

    fn parse_value<T>(&self, v: &[u8]) -> Result<T, MarketXmlError>
    where
        T: FromStr,
        T::Err: Display,
    {
        let s = self.decode_value(v)?;
        s.parse().map_err(|e| {
            MarketXmlError::Validation {
                msg: format!("{}", e),
                line: self.cur_line(),
                column: self.cur_column(),
                value: s.to_string(),
            }
        })
    }

    fn parse_opt<T>(&self, v: &[u8]) -> Result<Option<T>, MarketXmlError>
    where
        T: FromStr,
        T::Err: Display,
    {
        if v == b"" {
            Ok(None)
        } else {
            let s = self.decode_value(v)?;
            Some(
                s.parse().map_err(|e| {
                    MarketXmlError::Validation {
                        msg: format!("{}", e),
                        line: self.cur_line(),
                        column: self.cur_column(),
                        value: s.to_string(),
                    }
                })
            ).transpose()
        }
    }

    fn read_text_and_parse<F, T>(&mut self, f: F) -> Result<T, MarketXmlError>
    where
        F: FnOnce(&str, usize, usize) -> Result<T, MarketXmlError>,
    {
        let mut text = String::new();
        loop {
            match self.next_event()? {
                Event::Text(tag_text) |
                Event::CData(tag_text) => {
                    let bytes = tag_text.escaped();
                    match str::from_utf8(bytes) {
                        Ok(s) => text.push_str(s.trim()),
                        Err(e) => {
                            return Err(MarketXmlError::InvalidUtf8 {
                                msg: format!("{}", e),
                                value: String::from_utf8_lossy(bytes).to_string(),
                                line: self.cur_line(),
                                column: self.cur_column(),
                            });
                        }
                    }
                }
                Event::End(_) => {
                    break;
                }
                Event::Eof => return Err(MarketXmlError::Xml {
                    source: XmlError::UnexpectedEof("Text".to_string()),
                    line: self.cur_line(),
                    column: self.cur_column(),
                }),
                _ => return Err(MarketXmlError::Xml {
                    source: XmlError::TextNotFound,
                    line: self.cur_line(),
                    column: self.cur_column(),
                }),
            }
        }
        f(&text, self.cur_line(), self.cur_column())
    }
}


#[cfg(test)]
mod tests {
    use std::io::BufReader;

    use failure::{bail, Error};

    use crate::market_xml::{Category, Condition, Currency, DeliveryOption, Param};
    use super::{MarketXmlConfig, MarketXmlParser, ParsedItem};

    #[test]
    fn test_parsing_shop() -> Result<(), Error> {
        let xml = r#"
        <yml_catalog date="2019-11-01 17:22">
          <shop>
            <name>BestSeller</name>
            <company>Tne Best inc.</company>
            <url>http://best.seller.ru</url>
            <currencies>
              <currency id="RUR" rate="1"/>
              <currency id="USD" rate="60"/>
            </currencies>
            <categories>
              <category id="1">Бытовая техника</category>
              <category id="10" parentId="1">Мелкая техника для кухни</category>
            </categories>
            <delivery-options>
                <option cost="200" days="1"/>
            </delivery-options>
          </shop>
        </yml_catalog>
        "#;
        let reader = BufReader::new(xml.as_bytes());
        let mut parser = MarketXmlParser::new(
            MarketXmlConfig::default(),
            reader
        );
        let c = match parser.next_item()? {
            ParsedItem::YmlCatalog(yml_catalog) => yml_catalog,
            _ => bail!("Expected yml_catalog"),
        };
        assert_eq!(parser.current_line(), 19);
        assert_eq!(&c.date, "2019-11-01 17:22");
        let s = c.shop.unwrap();
        assert_eq!(&s.name, "BestSeller");
        assert_eq!(&s.company, "Tne Best inc.");
        assert_eq!(&s.url, "http://best.seller.ru");
        assert_eq!(
            s.currencies,
            vec!(
                Currency { id: "RUR".to_string(), rate: "1".to_string(), plus: "".to_string() },
                Currency { id: "USD".to_string(), rate: "60".to_string(), plus: "".to_string() },
            )
        );
        assert_eq!(
            s.categories,
            vec!(
                Category { id: 1, parent_id: 0, name: "Бытовая техника".to_string() },
                Category { id: 10, parent_id: 1, name: "Мелкая техника для кухни".to_string() }
            )
        );
        assert_eq!(
            s.delivery_options,
            vec!(
                DeliveryOption { cost: 200, days: "1".to_string(), order_before: None }
            )
        );

        Ok(())
    }

    #[test]
    fn test_parsing_simplified_offer() -> Result<(), Error> {
        let xml = r#"
        <yml_catalog>
          <shop>
            <name>Хладкомбинат</name>
            <offers>
              <offer id="9012" bid="80">
                <name>Мороженица Brand 3811</name>
                <vendor>Brand</vendor>
                <vendorCode>A1234567B</vendorCode>
                <url>http://best.seller.ru/product_page.asp?pid=12345</url>
                <price>8990</price>
                <oldprice>9990</oldprice>
                <enable_auto_discounts>true</enable_auto_discounts>
                <currencyId>RUR</currencyId>
                <categoryId>101</categoryId>
                <picture>http://best.seller.ru/img/model_12345.jpg</picture>
                <delivery>true</delivery>
                <pickup>true</pickup>
                <delivery-options>
                  <option cost="300" days="1" order-before="18"/>
                </delivery-options>
                <pickup-options>
                  <option cost="300" days="1-3"/>
                </pickup-options>
                <store>true</store>
                <description>
                  <![CDATA[
                    <h3>Мороженица Brand 3811</h3>
                    <p>Это прибор, который придётся по вкусу всем любителям десертов и сладостей, ведь с его помощью вы сможете делать вкусное домашнее мороженое из натуральных ингредиентов.</p>
                  ]]>
                </description>
                <sales_notes>Необходима предоплата.</sales_notes>
                <manufacturer_warranty>true</manufacturer_warranty>
                <country_of_origin>Китай</country_of_origin>
                <barcode>4601546021298</barcode>
                <param name="Цвет">белый</param>
                <condition type="likenew">
                    <reason>Повреждена упаковка</reason>
                </condition>
                <credit-template id="20034"/>
                <weight>3.6</weight>
                <dimensions>20.1/20.551/22.5</dimensions>
              </offer>
            </offers>
          </shop>
        </yml_catalog>
        "#;
        let reader = BufReader::new(xml.as_bytes());
        let mut parser = MarketXmlParser::new(
            MarketXmlConfig::default(),
            reader
        );
        let o = match parser.next_item()? {
            ParsedItem::Offer(offer) => offer,
            _ => bail!("Expected offer"),
        };
        assert_eq!(parser.current_line(), 43);
        assert_eq!(&o.id, "9012");
        assert_eq!(o.bid, 80);
        assert_eq!(&o.name, "Мороженица Brand 3811");
        assert_eq!(&o.vendor, "Brand");
        assert_eq!(&o.vendor_code, "A1234567B");
        assert_eq!(&o.url, "http://best.seller.ru/product_page.asp?pid=12345");
        assert_eq!(o.price.unwrap().price, 8990.0);
        assert_eq!(o.old_price.unwrap().price, 9990.0);
        assert_eq!(o.enable_auto_discounts, true);
        assert_eq!(&o.currency_id, "RUR");
        assert_eq!(o.category_id, 101);
        assert_eq!(&o.picture, "http://best.seller.ru/img/model_12345.jpg");
        assert_eq!(o.delivery, Some(true));
        assert_eq!(o.pickup, Some(true));
        assert_eq!(
            o.delivery_options,
            vec!(
                DeliveryOption { cost: 300, days: "1".to_string(), order_before: Some(18) }
            )
        );
        assert_eq!(
            o.pickup_options,
            vec!(
                DeliveryOption { cost: 300, days: "1-3".to_string(), order_before: None }
            )
        );
        assert_eq!(o.store, Some(true));
        assert_eq!(
            &o.description,
            r#"<h3>Мороженица Brand 3811</h3>
                    <p>Это прибор, который придётся по вкусу всем любителям десертов и сладостей, ведь с его помощью вы сможете делать вкусное домашнее мороженое из натуральных ингредиентов.</p>"#
        );
        assert_eq!(&o.sales_notes, "Необходима предоплата.");
        assert_eq!(o.manufacturer_warranty, true);
        assert_eq!(&o.country_of_origin, "Китай");
        assert_eq!(o.barcodes, vec!("4601546021298".to_string()));
        assert_eq!(
            o.params,
            vec!(
                Param {
                    name: "Цвет".to_string(),
                    unit: "".to_string(),
                    value: "белый".to_string(),
                    ..Default::default()
                }
            )
        );
        assert_eq!(o.condition, Some(Condition { r#type: "likenew".to_string(), reason: "Повреждена упаковка".to_string() }));
        assert_eq!(&o.credit_template_id, "20034");
        assert_eq!(o.weight, 3.6);
        assert_eq!(&o.dimensions, "20.1/20.551/22.5");

        let c = match parser.next_item()? {
            ParsedItem::YmlCatalog(yml_catalog) => yml_catalog,
            _ => bail!("Expected yml_catalog"),
        };
        assert_eq!(parser.current_line(), 46);
        assert_eq!(&c.date, "");
        let s = c.shop.unwrap();
        assert_eq!(&s.name, "Хладкомбинат");

        assert_eq!(parser.next_item()?, ParsedItem::Eof);

        Ok(())
    }
}
