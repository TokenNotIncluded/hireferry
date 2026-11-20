// Synthetic release records for UI/network tests; never distributed to users.
const repository='TokenNotIncluded/hireferry';
const rows=[
  ['cli','Windows','aarch64','aarch64-pc-windows-msvc','zip'],
  ['cli','Windows','x86_64','x86_64-pc-windows-msvc','zip'],
  ['cli','macOS','aarch64','aarch64-apple-darwin','tar.gz'],
  ['cli','macOS','x86_64','x86_64-apple-darwin','tar.gz'],
  ['cli','Linux','aarch64','aarch64-unknown-linux-musl','tar.gz'],
  ['cli','Linux','x86_64','x86_64-unknown-linux-musl','tar.gz'],
  ['server','Linux','aarch64','aarch64-unknown-linux-gnu','tar.gz'],
  ['server','Linux','x86_64','x86_64-unknown-linux-gnu','tar.gz'],
];
export const catalogFixture={schema_version:1,repository,versions:{cli:'0.1.3',server:'0.1.0'},releases:{cli:'cli-v0.1.3',server:'server-v0.1.0'},assets:rows.map(([product,platform,architecture,target,format],index)=>{
  const version=product==='cli'?'0.1.3':'0.1.0',tag=`${product}-v${version}`;
  const name=`hireferry-${product}-${version}-${target}.${format}`;
  return {product,platform,architecture,target,format,version,tag,name,size:2000000+index*1000,sha256:String(index).repeat(64),url:`https://github.com/${repository}/releases/download/${tag}/${name}`};
})};
