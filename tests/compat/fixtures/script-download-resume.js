(async () => {
  const {download} = processorHost.ProxyUtils;
  const text = await download(probeUpstream + '/text/reload');
  if (text !== '下载🙂\r\n\0尾\n') throw new Error('download cache lost on restart');
  const stored = $substore.read('downloadHelper');
  if (stored.text !== text || stored.zero !== 0 || stored.flag !== false || Object.keys(stored).length !== 3) throw new Error('script values lost on restart');
  return {storage: {cache: $substore.cache, root: $substore.root},
    resourceCache: processorHost.resourceCache.resourceCache,
    headersCache: processorHost.headersResourceCache.resourceCache};
})()
