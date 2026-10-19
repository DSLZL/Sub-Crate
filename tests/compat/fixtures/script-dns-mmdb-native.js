async function operator(proxies) {
  const assert=(value,message)=>{if(!value)throw new Error(message);};
  const equal=(actual,expected,message)=>assert(JSON.stringify(actual)===JSON.stringify(expected),message+': '+JSON.stringify({actual,expected}));
  const DateBefore=Date,random=Math.random;
  globalThis.Date=class extends DateBefore {constructor(...args){super(...(args.length?args:[3600000000000]));}static now(){return 3600000000000;}};
  const passed=[],check=async(id,run)=>{await run();passed.push(id);};
  const url='$UPSTREAM/host/dns/';
  try {
    await check('DN-native-packet-edns-and-doh',async()=>{
      const result=await ProxyUtils.doh({url:url+'good',domain:'native-doh.fixture',edns:'203.0.113.99',timeout:2000});
      equal(result.answers.filter(item=>item.type==='A').map(item=>item.data),['198.51.100.7','198.51.100.9'],'DoH packet');
      assert(result.rcode==='NOERROR','response flags');
    });
    await check('DN-native-cache-identity-and-winner',async()=>{
      const run=()=>DOMAIN_RESOLVERS.Custom('native-cache.fixture','IPv4',false,2000,undefined,url+'good',false,1,5000);
      const first=await run(),second=await run();
      equal(first,['198.51.100.7','198.51.100.9'],'cached answers');
      assert(first===second && second._resolverUrl===url+'good','live cached identity and resolver metadata');
      assert(!Object.keys(first).includes('_resolverUrl'),'winner metadata is not enumerable');
      const result=await DOMAIN_RESOLVERS.Custom('native-fallback.fixture','IPv4',false,2000,undefined,url+'empty\n'+url+'good',false,1);
      equal(result,first,'serial fallback');assert(result._resolverUrl===url+'good','fallback winner');
    });
    await check('DN-native-domain-processor-dedup-and-filters',async()=>{
      Math.random=()=>0;
      const input=[{name:'first',server:'native-operator.fixture'},{name:'second',server:'native-operator.fixture'},{name:'skip',server:'skip.fixture','no-resolve':true},{name:'literal',server:'192.0.2.1'}];
      const result=await ProxyUtils.process(input,[{type:'Resolve Domain Operator',args:{provider:'Custom',url:url+'good',type:'IPv4',filter:'IPv4Only',concurrency:1,dnsConcurrency:1,timeout:2000}}]);
      equal(result.map(item=>[item.name,item.server]),[['first','198.51.100.7'],['second','198.51.100.7'],['literal','192.0.2.1']],'ordered resolution and filter');
      assert(result[0]._resolved_ips===result[1]._resolved_ips && result[0]._domain==='native-operator.fixture' && result[0]._IPv4==='198.51.100.7','resolved annotations');
      equal(input.map(item=>item.server),['native-operator.fixture','native-operator.fixture','skip.fixture','192.0.2.1'],'operator clones input');
    });
    await check('DN-native-mmdb-and-public-reader-methods',async()=>{
      const db=new ProxyUtils.MMDB();
      equal([db.geoip('81.2.69.160'),db.geoip('2001:218::1'),db.ipasn('2001:1700::1'),db.ipaso('2001:1700::1')],['GB','JP',6730,'Sunrise Communications AG'],'native country and ASN');
      db.countryReader={country:()=>({country:{isoCode:false}})};db.asnReader={asn:()=>({autonomousSystemNumber:0,autonomousSystemOrganization:''})};
      equal([db.geoip('x'),db.ipasn('x'),db.ipaso('x')],[false,0,''],'public readers preserve falsy values');
      delete db.countryReader;delete db.asnReader;
      assert(db.geoip('x')===undefined && db.ipasn('x')===undefined,'optional readers');
      let rejected=false;try{new ProxyUtils.MMDB(null);}catch(error){rejected=error.name==='TypeError';}assert(rejected,'invalid options');
    });
    $substore.write({passed},'m3-native-dns-mmdb');
    return proxies.map(proxy=>({...proxy,dnsMmdbVerified:passed.length}));
  } catch(error) {
    return proxies.map(proxy=>({...proxy,dnsMmdbError:String(error),dnsMmdbPassed:passed}));
  } finally {globalThis.Date=DateBefore;Math.random=random;}
}
