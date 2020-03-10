use failure::{Error, format_err};

use quick_xml::{Reader as XmlReader, Error as XmlError};
use quick_xml::events::{Event, BytesStart};
use quick_xml::events::attributes::Attributes;

use std::io::prelude::BufRead;

use std::borrow::Cow;

use crate::market_xml::{DeliveryOption, Offer, offer};

struct MarketXmlConfig {

}

impl Default for MarketXmlConfig {
    fn default() -> Self {
        Self {}
    }
}

#[derive(PartialEq, Clone, Copy)]
enum State {
    None,
    Catalog,
    Shop,
    Offers,
}

struct MarketXmlParser<B: BufRead> {
    config: MarketXmlConfig,
    xml_reader: XmlReader<B>,
    buf: Vec<u8>,
    stack: Vec<String>,
    state: State,
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
            state: State::None,
        }
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

    fn parse_credit_template(&self, tag_attrs: &mut Attributes) -> Result<Option<String>, Error> {
        for attr_res in tag_attrs {
            let attr = attr_res?;
            if attr.key == b"id" {
                return Ok(Some(String::from_utf8_lossy(&attr.value).to_string()));
            }
        }
        Ok(None)
    }

    fn read_text(&mut self) -> Result<String, XmlError> {
        let mut text = String::new();
        loop {
            match self.xml_reader.read_event(&mut self.buf)? {
                Event::Text(tag_text) |
                Event::CData(tag_text) => {
                    let bytes = &tag_text.unescaped()?.into_owned();
                    text.push_str(&String::from_utf8_lossy(&bytes).trim());
                }
                Event::End(tag) => {
                    break;
                }
                Event::Eof => return Err(XmlError::UnexpectedEof("Text".to_string())),
                _ => return Err(XmlError::TextNotFound),
            }
        }
        Ok(text)
    }

    fn read_bool(&mut self) -> Result<bool, Error> {
        return self.read_text_and_map(|t| Ok(t.parse()?))
    }

    fn read_f64(&mut self) -> Result<f64, Error> {
        return self.read_text_and_map(|t| Ok(t.parse()?))
    }

    fn read_f32(&mut self) -> Result<f32, Error> {
        return self.read_text_and_map(|t| Ok(t.parse()?))
    }

    fn read_u64(&mut self) -> Result<u64, Error> {
        return self.read_text_and_map(|t| Ok(t.parse()?))
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
                Event::End(tag) => {
                    break;
                }
                Event::Eof => return Err(XmlError::UnexpectedEof("Text".to_string()).into()),
                _ => return Err(XmlError::TextNotFound.into()),
            }
        }
        f(&text)
    }

    fn parse_price(&mut self, tag_attrs: &mut Attributes) -> Result<offer::Price, Error> {
        let mut price = offer::Price::default();
        price.price = self.read_f32()?;
        for attr_res in tag_attrs {
            let attr = attr_res?;
            if attr.key == b"from" && attr.value.as_ref() == b"true" {
                price.from = true;
            }
        }
        Ok(price)
    }

    fn parse_param(&mut self, tag_attrs: &mut Attributes) -> Result<offer::Param, Error> {
        let mut param = offer::Param::default();
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

    fn parse_condition(&mut self, tag_attrs: &mut Attributes) -> Result<offer::Condition, Error> {
        let mut condition = offer::Condition::default();
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
}

impl<B: BufRead> Iterator for MarketXmlParser<B> {
    type Item = Result<(), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.xml_reader.read_event(&mut self.buf) {
                Ok(Event::Start(ref tag)) => {
                    let tag_name = String::from_utf8_lossy(tag.name());
                    self.stack.push(tag_name.to_string());
                    println!("> {}", self.stack.join("."));
                    match tag_name.as_ref() {
                        "yml_catalog" if self.state == State::None => {
                            self.state = State::Catalog;
                        }
                        "shop" if self.state == State::Catalog => {
                            self.state = State::Shop;
                        }
                        "offers" if self.state == State::Shop => {
                            self.state = State::Offers;
                        }
                        "offer" if self.state == State::Offers => {
                            let mut offer = Offer::default();
                            let tag = tag.to_owned();
                            self.parse_offer_attributes(&mut tag.attributes(), &mut offer);
                            self.parse_offer_fields(&mut offer);
                            println!("offer: {:?}", &offer);
                        }
                        _ => {
                            return Some(Err(format_err!("Unexpected tag: {}", tag_name)));
                        }
                    }
                    return Some(Ok(()));
                }
                Ok(Event::Empty(ref tag)) => {
                    let tag_name = String::from_utf8_lossy(tag.name());
                    println!("= {}.{}", self.stack.join("."), tag_name);
                    return Some(Ok(()));
                }
                Ok(Event::End(ref tag)) => {
                    println!("< {}", self.stack.join("."));
                    self.stack.pop();
                    match tag.name() {
                        b"offer" if self.state == State::Offers => {}
                        b"offers" if self.state == State::Offers => {
                            self.state = State::Shop;
                        }
                        b"shop" if self.state == State::Shop => {
                            self.state = State::Catalog;
                        }
                        b"yml_catalog" if self.state == State::Catalog => {
                            self.state = State::None;
                        }
                        _ => {
                            return Some(Err(
                                format_err!("Unexpected close tag: {}", String::from_utf8_lossy(tag.name()))
                            ))
                        }
                    }
                    return Some(Ok(()));
                }
                Ok(Event::Eof) => {
                    return None;
                }
                Err(e) => {
                    return Some(Err(e.into()));
                }
                _ => {}
            }

            self.buf.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::BufReader;

    use super::{MarketXmlConfig, MarketXmlParser};

    #[test]
    fn test_market_xml_parser() {
        let xml = r#"
<yml_catalog><shop><offers>
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
</offers></shop></yml_catalog>
        "#;
        let reader = BufReader::new(xml.as_bytes());
        let mut parser = MarketXmlParser::new(
            MarketXmlConfig {},
            reader
        );
        for it in parser {

        }
        unimplemented!()
    }
}
