use quick_xml::{Reader as XmlReader, Error as XmlError};
use quick_xml::events::{Event, BytesStart};
use quick_xml::events::attributes::Attributes;

use snafu::{ResultExt, Snafu};

use std::collections::HashSet;
use std::io::prelude::BufRead;
use std::num::{ParseFloatError, ParseIntError};
use std::str::ParseBoolError;

use crate::market_xml::{Category, Condition, Currency, DeliveryOption, Offer, Param, Price, Shop};


#[derive(Debug, Snafu)]
pub(crate) enum MarketXmlError {
    #[snafu(display("Xml error: {}", source))]
    Xml {
        source: XmlError,
        line: usize,
    },
    #[snafu(display("Unexpected tag: {}", tag))]
    UnexpectedTag {
        tag: String,
        line: usize,
    },
    #[snafu(display("{}", source))]
    ParseBool {
        source: ParseBoolError,
        line: usize,
        value: String,
    },
    #[snafu(display("{}", source))]
    ParseFloat {
        source: ParseFloatError,
        line: usize,
        value: String,
    },
    #[snafu(display("{}", source))]
    ParseInt {
        source: ParseIntError,
        line: usize,
        value: String,
    },
}

impl MarketXmlError {
    pub(crate) fn line(&self) -> usize {
        use MarketXmlError::*;

        match *self {
            Xml { line, .. } => line,
            UnexpectedTag { line, .. } => line,
            ParseBool { line, .. } => line,
            ParseFloat { line, .. } => line,
            ParseInt { line, .. } => line,
        }
    }

    pub(crate) fn value(&self) -> Option<&str> {
        use MarketXmlError::*;

        match self {
            Xml { .. } => None,
            UnexpectedTag { tag, .. } => Some(tag),
            ParseBool { value, .. } => Some(value),
            ParseFloat { value, .. } => Some(value),
            ParseInt { value, .. } => Some(value),
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
    xml_reader: XmlReader<B>,
    buf: Vec<u8>,
    state: State,
    shop: Shop,
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
    Shop(Shop),
}

impl<B: BufRead> MarketXmlParser<B> {
    pub(crate) fn new(config: MarketXmlConfig, reader: B) -> Self {
        let mut xml_reader = XmlReader::from_reader(reader);
        xml_reader.trim_text(true);
        Self {
            config,
            xml_reader,
            buf: vec!(),
            state: State::Begin,
            shop: Shop::default(),
        }
    }

    fn current_line(&self) -> usize {
        self.xml_reader.line_number()
    }

    fn xml_err_ctx(&self) -> Xml<usize> {
        Xml {
            line: self.xml_reader.line_number(),
        }
    }

    fn next_event(&mut self) -> Result<Event, MarketXmlError> {
        let event_res = self.xml_reader.read_event(&mut self.buf);
        match event_res {
            Ok(event) => Ok(event),
            Err(error) => {
                Err(MarketXmlError::Xml {
                    source: error,
                    line: self.xml_reader.line_number(),
                })
            }
        }
    }

    pub(crate) fn next_item(&mut self) -> Option<Result<ParsedItem, MarketXmlError>> {
        loop {
            match self.state {
                State::Begin => {
                    match self.begin() {
                        Ok(state) => self.state = state,
                        Err(e) => return Some(Err(e)),
                    }
                }
                State::YmlCatalog => {
                    match self.parse_yml_catalog() {
                        Ok(state) => self.state = state,
                        Err(e) => return Some(Err(e)),
                    }
                }
                State::Shop => {
                    match self.parse_shop() {
                        Ok(state) => {
                            self.state = state;
                            if state == State::YmlCatalog {
                                return Some(Ok(ParsedItem::Shop(self.shop.clone())));
                            }
                        }
                        Err(e) => return Some(Err(e)),
                    }
                }
                State::Offers => {
                    match self.parse_offers() {
                        Ok(Some(offer)) => {
                            return Some(Ok(ParsedItem::Offer(offer)));
                        }
                        Ok(None) => {
                            self.state = State::Shop;
                        }
                        Err(e) => return Some(Err(e)),
                    }
                }
                State::End => {
                    return None;
                }
            }
        }
    }

    fn begin(&mut self) -> Result<State, MarketXmlError> {
        loop {
            match self.next_event()? {
                Event::Start(tag) => {
                    if tag.name() == b"yml_catalog" {
                        return Ok(State::YmlCatalog);
                    }
                    return Err(MarketXmlError::UnexpectedTag {
                        tag: String::from_utf8_lossy(tag.name()).to_string(),
                        line: self.current_line(),
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

    fn parse_shop(&mut self) -> Result<State, MarketXmlError> {
        Ok(loop {
            match self.next_event()? {
                Event::Start(tag) |
                Event::Empty(tag) => {
                    if tag.name() == b"offers" {
                        break State::Offers;
                    }
                    let tag = tag.to_owned();
                    self.parse_shop_field(tag)?;
                }
                Event::End(tag) => {
                    if tag.name() == b"shop" {
                        break State::YmlCatalog;
                    }
                }
                Event::Eof => {
                    return Err(XmlError::UnexpectedEof("shop".to_string()))
                        .context(self.xml_err_ctx());
                },
                _ => {}
            }
        })
    }

    fn parse_shop_field(&mut self, tag: BytesStart) -> Result<(), MarketXmlError> {
        match tag.name() {
            b"name" => {
                self.shop.name = self.read_text()?;
            }
            b"company" => {
                self.shop.company = self.read_text()?;
            }
            b"url" => {
                self.shop.url = self.read_text()?;
            }
            b"currencies" => {
                self.shop.currencies = self.parse_currencies()?;
            }
            b"categories" => {
                self.shop.categories = self.parse_categories()?;
            }
            b"delivery-options" => {
                self.shop.delivery_options = self.parse_delivery_options()?;
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
            let value = String::from_utf8_lossy(&attr.value).to_string();
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
            let value = String::from_utf8_lossy(&attr.value);
            match attr.key {
                b"id" => {
                    category.id = self.parse_u64(value.as_ref())?;
                }
                b"parentId" => {
                    category.parent_id = self.parse_u64(value.as_ref())?;
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
                    offer.id = String::from_utf8_lossy(&attr.value).to_string();
                }
                b"type" => {
                    offer.r#type = String::from_utf8_lossy(&attr.value).to_string();
                }
                b"bid" => {
                    offer.bid = self.parse_u32(&String::from_utf8_lossy(&attr.value))?
                }
                b"cbid" => {
                    offer.cbid = self.parse_u32(&String::from_utf8_lossy(&attr.value))?
                }
                b"available" => {
                    offer.available = match attr.value.as_ref() {
                        b"" | b"false" | b"0" => false,
                        b"true" | b"1" => true,
                        _ => false,
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
                offer.category_id = self.read_u64()?;
            }
            b"description" => {
                offer.description = self.read_text()?;
            }
            b"sales_notes" => {
                offer.sales_notes = self.read_text()?;
            }
            b"delivery" => {
                offer.delivery = self.read_bool()?;
            }
            b"pickup" => {
                offer.pickup = self.read_bool()?;
            }
            b"store" => {
                offer.store = self.read_bool()?;
            }
            b"downloadable" => {
                offer.downloadable = self.read_bool()?;
            }
            b"enable_auto_discounts" => {
                offer.enable_auto_discounts = self.read_bool()?;
            }
            b"manufacturer_warranty" => {
                offer.manufacturer_warranty = self.read_bool()?;
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
                offer.weight = self.read_f32()?;
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
                println!("> {}", String::from_utf8_lossy(tag.name()));
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
                    option.cost = self.parse_u32(&String::from_utf8_lossy(&attr.value))?;
                }
                b"days" => {
                    option.days = String::from_utf8_lossy(&attr.value).to_string();
                }
                b"order-before" => {
                    option.order_before = self.parse_u32(&String::from_utf8_lossy(&attr.value))?;
                }
                _ => {}
            }
        }
        Ok(option)
    }

    fn parse_price(&mut self, tag_attrs: &mut Attributes) -> Result<Price, MarketXmlError> {
        let mut price = Price::default();
        price.price = self.read_f32()?;
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
            match attr.key {
                b"name" => {
                    param.name = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                }
                b"unit" => {
                    param.unit = String::from_utf8_lossy(attr.value.as_ref()).to_string();
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
                    condition.r#type = String::from_utf8_lossy(attr.value.as_ref()).to_string();
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
                return Ok(Some(String::from_utf8_lossy(&attr.value).to_string()));
            }
        }
        Ok(None)
    }

    fn read_text(&mut self) -> Result<String, MarketXmlError> {
        self.read_text_and_parse(|s, _| Ok(s.to_string()))
    }

    fn read_bool(&mut self) -> Result<bool, MarketXmlError> {
        self.read_text_and_parse(|s, line| {
            s.parse().context(ParseBool { line, value: s.to_string() })
        })
    }

    fn read_f64(&mut self) -> Result<f64, MarketXmlError> {
        self.read_text_and_parse(|s, line| {
            s.parse().context(ParseFloat { line, value: s.to_string() })
        })
    }

    fn read_f32(&mut self) -> Result<f32, MarketXmlError> {
        self.read_text_and_parse(|s, line| {
            s.parse().context(ParseFloat { line, value: s.to_string() })
        })

    }

    fn read_u64(&mut self) -> Result<u64, MarketXmlError> {
        self.read_text_and_parse(|s, line| {
            s.parse().context(ParseInt { line, value: s.to_string() })
        })
    }

    fn parse_u64(&self, s: &str) -> Result<u64, MarketXmlError> {
        s.parse().context(ParseInt { line: self.current_line(), value: s.to_string() })
    }

    fn parse_u32(&self, s: &str) -> Result<u32, MarketXmlError> {
        s.parse().context(ParseInt { line: self.current_line(), value: s.to_string() })
    }

    fn read_text_and_parse<F, T>(&mut self, f: F) -> Result<T, MarketXmlError>
    where
        F: FnOnce(&str, usize) -> Result<T, MarketXmlError>,
    {
        let mut text = String::new();
        loop {
            match self.next_event()? {
                Event::Text(tag_text) |
                Event::CData(tag_text) => {
                    let bytes = tag_text.escaped();
                    text.push_str(&String::from_utf8_lossy(&bytes).trim());
                }
                Event::End(_) => {
                    break;
                }
                Event::Eof => return Err(MarketXmlError::Xml {
                    source: XmlError::UnexpectedEof("Text".to_string()),
                    line: self.current_line(),
                }),
                _ => return Err(MarketXmlError::Xml {
                    source: XmlError::TextNotFound,
                    line: self.current_line(),
                }),
            }
        }
        f(&text, self.current_line())
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
        let s = match parser.next_item().unwrap()? {
            ParsedItem::Shop(shop) => shop,
            _ => bail!("Expected shop"),
        };
        assert_eq!(parser.current_line(), 18);
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
                DeliveryOption { cost: 200, days: "1".to_string(), order_before: 0 }
            )
        );

        Ok(())
    }

    #[test]
    fn test_parsing_simplified_offer() -> Result<(), Error> {
        let xml = r#"
        <yml_catalog>
          <shop>
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
        let o = match parser.next_item().unwrap()? {
            ParsedItem::Offer(offer) => offer,
            _ => bail!("Expected offer"),
        };
        assert_eq!(parser.current_line(), 42);
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
        assert_eq!(o.delivery, true);
        assert_eq!(o.pickup, true);
        assert_eq!(
            o.delivery_options,
            vec!(
                DeliveryOption { cost: 300, days: "1".to_string(), order_before: 18 }
            )
        );
        assert_eq!(
            o.pickup_options,
            vec!(
                DeliveryOption { cost: 300, days: "1-3".to_string(), order_before: 0 }
            )
        );
        assert_eq!(o.store, true);
        assert_eq!(
            &o.description,
            r#"<h3>Мороженица Brand 3811</h3>
                    <p>Это прибор, который придётся по вкусу всем любителям десертов и сладостей, ведь с его помощью вы сможете делать вкусное домашнее мороженое из натуральных ингредиентов.</p>"#
        );
        assert_eq!(&o.sales_notes, "Необходима предоплата.");
        assert_eq!(o.manufacturer_warranty, true);
        assert_eq!(&o.country_of_origin, "Китай");
        assert_eq!(o.barcodes, vec!("4601546021298".to_string()));
        assert_eq!(o.params, vec!(Param { name: "Цвет".to_string(), unit: "".to_string(), value: "белый".to_string() }));
        assert_eq!(o.condition, Some(Condition { r#type: "likenew".to_string(), reason: "Повреждена упаковка".to_string() }));
        assert_eq!(&o.credit_template_id, "20034");
        assert_eq!(o.weight, 3.6);
        assert_eq!(&o.dimensions, "20.1/20.551/22.5");

        let s = match parser.next_item().unwrap()? {
            ParsedItem::Shop(shop) => shop,
            _ => bail!("Expected shop"),
        };
        assert_eq!(parser.current_line(), 44);
        assert_eq!(&s.name, "");

        println!("{:?}", parser.next_item());
        assert!(parser.next_item().is_none());

        Ok(())
    }
}
