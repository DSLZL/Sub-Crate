use crate::{Result, require};
use base64::Engine;
use std::net::IpAddr;

pub fn name_bytes(domain: &str) -> Result<Vec<u8>> {
    let mut result=Vec::new();
    for label in domain.split('.') { require(label.is_ascii() && !label.is_empty() && label.len()<=63,"Invalid fixture DNS label")?; result.push(label.len() as u8); result.extend_from_slice(label.as_bytes()); }
    result.push(0); Ok(result)
}

fn words(values: &[u16]) -> Vec<u8> { values.iter().flat_map(|word| word.to_be_bytes()).collect() }

pub fn query_bytes(domain: &str, kind: u16, edns: Option<IpAddr>) -> Result<Vec<u8>> {
    let mut additional=Vec::new();
    if let Some(address)=edns {
        let (family,prefix,packed)=match address { IpAddr::V4(address)=>(1,24,address.octets().to_vec()),IpAddr::V6(address)=>(2,56,address.octets().to_vec()) };
        let mut subnet=words(&[family]); subnet.extend([prefix,0]); subnet.extend_from_slice(&packed[..usize::from(prefix)/8]);
        let mut option=words(&[8,subnet.len() as u16]); option.extend(subnet);
        additional.push(0); additional.extend(words(&[41,4096,0,0,option.len() as u16])); additional.extend(option);
    }
    let mut packet=words(&[0,0x100,1,0,0,u16::from(!additional.is_empty())]); packet.extend(name_bytes(domain)?); packet.extend(words(&[kind,1])); packet.extend(additional); Ok(packet)
}

pub fn query_path(mode: &str, domain: &str, kind: u16, edns: Option<IpAddr>) -> Result<String> {
    Ok(format!("/host/dns/{mode}?dns={}",base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(query_bytes(domain,kind,edns)?)))
}

pub fn response_bytes(packet: &[u8], mode: &str) -> Result<Vec<u8>> {
    require(packet.len()>=12,"Truncated DNS header")?;
    let mut offset=12;
    while let Some(length)=packet.get(offset).copied().filter(|length| *length!=0) { require(length<=63,"Fixture accepts uncompressed labels only")?; offset+=usize::from(length)+1; }
    let end=offset+5; require(end<=packet.len(),"Truncated DNS question")?;
    let kind=u16::from_be_bytes([packet[offset+1],packet[offset+2]]);
    require(packet[..10]==words(&[0,0x100,1,0,0]) && packet[offset+3..end]==[0,1],"Unexpected fixture DNS query header")?;
    if mode=="malformed" { return Ok(vec![0,1]); }
    let mut records=Vec::new();
    if !["empty","serial-empty","nxdomain"].contains(&mode) {
        records.push((5,name_bytes("alias.fixture")?));
        let answer_kind=if mode=="wrong" { 28 } else { kind };
        let addresses=if answer_kind==28 { vec!["2001:db8::7"] } else { vec!["198.51.100.7","198.51.100.9"] };
        for address in addresses { let data=match address.parse::<IpAddr>()? { IpAddr::V4(ip)=>ip.octets().to_vec(),IpAddr::V6(ip)=>ip.octets().to_vec() }; records.push((answer_kind,data)); }
    }
    let flags=if ["nxdomain","rcode"].contains(&mode) { 0x8183 } else { 0x8180 };
    let mut response=words(&[0,flags,1,records.len() as u16,0,0]); response.extend_from_slice(&packet[12..end]);
    for (kind,data) in records { response.extend([0xc0,0x0c]); response.extend(words(&[kind,1,0,60,data.len() as u16])); response.extend(data); }
    Ok(response)
}
