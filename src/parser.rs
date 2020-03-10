use failure::{Error, format_err};

use quick_xml::{Reader as XmlReader, Error as XmlError};
use quick_xml::events::{Event, BytesStart};
use quick_xml::events::attributes::Attributes;

use std::collections::HashSet;
use std::io::prelude::BufRead;

use crate::market_xml::{Category, Condition, Currency, DeliveryOption, Offer, Param, Price, Shop};

struct MarketXmlConfig {
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

struct MarketXmlParser<B: BufRead> {
    config: MarketXmlConfig,
    xml_reader: XmlReader<B>,
    buf: Vec<u8>,
    stack: Vec<String>,
    state: State,
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum State {
    Begin,
    YmlCatalog,
    Shop,
    Offers,
    End,
}

enum ParsedItem {
    Offer(Offer),
    Shop(Shop),
}

impl<B: BufRead> Iterator for MarketXmlParser<B> {
    type Item = Result<ParsedItem, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.state = match self.state {
                State::Begin => {
                    match self.begin() {
                        Ok(state) => state,
                        Err(e) => return Some(Err(e)),
                    }
                }
                State::YmlCatalog => {
                    match self.parse_yml_catalog() {
                        Ok(state) => state,
                        Err(e) => return Some(Err(e)),
                    }
                }
                State::Shop => {
                    let mut shop = Shop::default();
                    match self.parse_shop(&mut shop) {
                        Ok(state) => {
                            if state == State::YmlCatalog {
                                return Some(Ok(ParsedItem::Shop(shop)));
                            }
                            state
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
                            State::Shop
                        }
                        Err(e) => return Some(Err(e)),
                    }
                }
                State::End => {
                    return None;
                }
            };
        }
    }
}

impl<B: BufRead> MarketXmlParser<B> {
    fn new(config: MarketXmlConfig, reader: B) -> Self {
        let mut xml_reader = XmlReader::from_reader(reader);
        xml_reader.trim_text(true);
        Self {
            config,
            xml_reader,
            buf: vec!(),
            stack: vec!(),
            state: State::Begin,
        }
    }

    fn begin(&mut self) -> Result<State, Error> {
        loop {
            match self.xml_reader.read_event(&mut self.buf)? {
                Event::Start(tag) => {
                    if tag.name() == b"yml_catalog" {
                        return Ok(State::YmlCatalog);
                    }
                    return Err(format_err!(
                        "Unexpected tag: {}", String::from_utf8_lossy(tag.name())
                    ));
                }
                Event::Eof => {
                    return Err(XmlError::UnexpectedEof("yandex market file".to_string()).into());
                }
                _ => {}
            }
        }
    }

    fn parse_yml_catalog(&mut self) -> Result<State, Error> {
        loop {
            match self.xml_reader.read_event(&mut self.buf)? {
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
                    return Err(XmlError::UnexpectedEof("yml_catalog".to_string()).into());
                }
                _ => {}
            }
        }
    }

    fn parse_shop(&mut self, shop: &mut Shop) -> Result<State, Error> {
        Ok(loop {
            match self.xml_reader.read_event(&mut self.buf)? {
                Event::Start(tag) |
                Event::Empty(tag) => {
                    if tag.name() == b"offers" {
                        break State::Offers;
                    }
                    let tag = tag.to_owned();
                    self.parse_shop_field(shop, tag)?;
                }
                Event::End(tag) => {
                    let tag_name = tag.name();
                    if tag_name == b"shop" {
                        break State::YmlCatalog;
                    }
                }
                Event::Eof => return Err(XmlError::UnexpectedEof("shop".to_string()).into()),
                _ => {}
            }
        })
    }

    fn parse_shop_field(&mut self, shop: &mut Shop, tag: BytesStart) -> Result<(), Error> {
        match tag.name() {
            b"name" => {
                shop.name = self.read_text()?;
            }
            b"company" => {
                shop.company = self.read_text()?;
            }
            b"url" => {
                shop.url = self.read_text()?;
            }
            b"currencies" => {
                shop.currencies = self.parse_currencies()?;
            }
            b"categories" => {
                shop.categories = self.parse_categories()?;
            }
            b"delivery-options" => {
                shop.delivery_options = self.parse_delivery_options()?;
            }
            _ => {}
        }
        Ok(())
    }

    fn parse_currencies(&mut self) -> Result<Vec<Currency>, Error> {
        let mut currencies = vec!();
        loop {
            match self.xml_reader.read_event(&mut self.buf)? {
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
                Event::Eof => Err(XmlError::UnexpectedEof("currencies".to_string()))?,
                _ => {}
            }
        }
    }

    fn parse_currency(&mut self, attrs: &mut Attributes) -> Result<Currency, Error> {
        let mut currency = Currency::default();
        for attr_res in attrs {
            let attr = attr_res?;
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

    fn parse_categories(&mut self) -> Result<Vec<Category>, Error> {
        let mut categories = vec!();
        loop {
            match self.xml_reader.read_event(&mut self.buf)? {
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
                Event::Eof => Err(XmlError::UnexpectedEof("categories".to_string()))?,
                _ => {}
            }
        }
    }

    fn parse_category(&mut self, attrs: &mut Attributes) -> Result<Category, Error> {
        let mut category = Category::default();
        for attr_res in attrs {
            let attr = attr_res?;
            let value = String::from_utf8_lossy(&attr.value);
            match attr.key {
                b"id" => {
                    category.id = value.parse()?;
                }
                b"parentId" => {
                    category.parent_id = value.parse()?;
                }
                _ => {}
            }
        }
        category.name = self.read_text()?;
        Ok(category)
    }

    fn parse_offers(&mut self) -> Result<Option<Offer>, Error> {
        loop {
            match self.xml_reader.read_event(&mut self.buf)? {
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
                Event::Eof => Err(XmlError::UnexpectedEof("offers".to_string()))?,
                _ => {}
            }

            self.buf.clear();
        }
    }

    fn parse_offer(&mut self, attrs: &mut Attributes) -> Result<Offer, Error> {
        let mut offer = Offer::default();
        self.parse_offer_attributes(attrs, &mut offer)?;
        self.parse_offer_fields(&mut offer)?;
        Ok(offer)
    }

    fn parse_offer_attributes(
        &self, attrs: &mut Attributes, offer: &mut Offer
    ) -> Result<(), Error> {
        for attr_res in attrs {
            let attr = attr_res?;
            match attr.key {
                b"id" => {
                    offer.id = String::from_utf8_lossy(&attr.value).to_string();
                }
                b"type" => {
                    offer.r#type = String::from_utf8_lossy(&attr.value).to_string();
                }
                b"bid" => {
                    offer.bid = String::from_utf8_lossy(&attr.value).to_string().parse()?;
                }
                b"cbid" => {
                    offer.cbid = String::from_utf8_lossy(&attr.value).to_string().parse()?;
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

    fn parse_offer_fields(&mut self, offer: &mut Offer) -> Result<(), Error> {
        loop {
            let event = self.xml_reader.read_event(&mut self.buf)?;
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
                Event::Eof => return Err(XmlError::UnexpectedEof("Offer".to_string()).into()),
                _ => {}
            }
        }
        Ok(())
    }

    fn parse_offer_field(&mut self, tag: BytesStart, offer: &mut Offer) -> Result<(), Error> {
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

    fn parse_delivery_options(&mut self) -> Result<Vec<DeliveryOption>, Error> {
        let mut options = vec!();
        loop {
            match self.xml_reader.read_event(&mut self.buf)? {
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
                Event::Eof => Err(XmlError::UnexpectedEof("Delivery options".to_string()))?,
                _ => Err(XmlError::TextNotFound)?,
            }
        }
        Ok(options)
    }

    fn parse_delivery_option(&self, tag_attrs: &mut Attributes) -> Result<DeliveryOption, Error> {
        let mut option = DeliveryOption::default();
        for attr_res in tag_attrs {
            let attr = attr_res?;
            match attr.key {
                b"cost" => {
                    option.cost = String::from_utf8_lossy(&attr.value).parse()?;
                }
                b"days" => {
                    option.days = String::from_utf8_lossy(&attr.value).to_string();
                }
                b"order-before" => {
                    option.order_before = String::from_utf8_lossy(&attr.value).parse()?;
                }
                _ => {}
            }
        }
        Ok(option)
    }

    fn parse_price(&mut self, tag_attrs: &mut Attributes) -> Result<Price, Error> {
        let mut price = Price::default();
        price.price = self.read_f32()?;
        for attr_res in tag_attrs {
            let attr = attr_res?;
            if attr.key == b"from" && attr.value.as_ref() == b"true" {
                price.from = true;
            }
        }
        Ok(price)
    }

    fn parse_param(&mut self, tag_attrs: &mut Attributes) -> Result<Param, Error> {
        let mut param = Param::default();
        param.value = self.read_text()?;
        for attr_res in tag_attrs {
            let attr = attr_res?;
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

    fn parse_condition(&mut self, tag_attrs: &mut Attributes) -> Result<Condition, Error> {
        let mut condition = Condition::default();
        for attr_res in tag_attrs {
            let attr = attr_res?;
            match attr.key {
                b"type" => {
                    condition.r#type = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                }
                _ => {}
            }
        }
        loop {
            match self.xml_reader.read_event(&mut self.buf)? {
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
                Event::Eof => return Err(XmlError::UnexpectedEof("Condition".to_string()).into()),
                _ => return Err(XmlError::TextNotFound.into()),
            }
        }
        Ok(condition)
    }

    fn parse_credit_template(&self, tag_attrs: &mut Attributes) -> Result<Option<String>, Error> {
        for attr_res in tag_attrs {
            let attr = attr_res?;
            if attr.key == b"id" {
                return Ok(Some(String::from_utf8_lossy(&attr.value).to_string()));
            }
        }
        Ok(None)
    }

    fn read_text(&mut self) -> Result<String, Error> {
        self.read_text_and_map(|t| Ok(t.to_string()))
    }

    fn read_bool(&mut self) -> Result<bool, Error> {
        self.read_text_and_map(|t| Ok(t.parse()?))
    }

    fn read_f64(&mut self) -> Result<f64, Error> {
        self.read_text_and_map(|t| Ok(t.parse()?))
    }

    fn read_f32(&mut self) -> Result<f32, Error> {
        self.read_text_and_map(|t| Ok(t.parse()?))
    }

    fn read_u64(&mut self) -> Result<u64, Error> {
        self.read_text_and_map(|t| Ok(t.parse()?))
    }

    fn read_text_and_map<F, T>(&mut self, f: F) -> Result<T, Error>
    where F: FnOnce(&str) -> Result<T, Error>
    {
        let mut text = String::new();
        loop {
            match self.xml_reader.read_event(&mut self.buf)? {
                Event::Text(tag_text) |
                Event::CData(tag_text) => {
                    let bytes = &tag_text.unescaped()?.into_owned();
                    text.push_str(&String::from_utf8_lossy(&bytes).trim());
                }
                Event::End(_) => {
                    break;
                }
                Event::Eof => return Err(XmlError::UnexpectedEof("Text".to_string()).into()),
                _ => return Err(XmlError::TextNotFound.into()),
            }
        }
        f(&text)
    }
}


#[cfg(test)]
mod tests {
    use failure::{Error, format_err};

    use std::io::BufReader;

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
        let s = match parser.next().unwrap()? {
            ParsedItem::Shop(o) => Ok(o),
            _ => Err(format_err!("Expected shop"))
        }?;
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
        let o = match parser.next().unwrap()? {
            ParsedItem::Offer(o) => Ok(o),
            _ => Err(format_err!("Expected offer"))
        }?;
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

        let s = match parser.next().unwrap()? {
            ParsedItem::Shop(o) => Ok(o),
            _ => Err(format_err!("Expected shop"))
        }?;
        assert_eq!(&s.name, "");

        Ok(())
    }
}
