#[path="../../../src/subtitles.rs"] mod subtitles;
fn main() {
 let mut total=0usize;
 for (folder,names) in [
  ("sami-sync",["clear","replace","quoted","lines","language-gap","invalid-clock"]),
  ("sami-body-boundaries",["quoted-body-close","comment-body-close","title-raw-close","style-raw-close","title-control","comment-control"]),
 ] {
  let root=std::path::Path::new("tests/fixtures/playback-errors").join(folder);
  for name in names {
   let text=std::fs::read_to_string(root.join(format!("{name}.smi"))).unwrap();
   assert!(!subtitles::parse_sami(&text).is_empty(),"valid seed {folder}/{name}");
   let mut boundaries:Vec<_>=text.char_indices().map(|(i,_)|i).collect();boundaries.push(text.len());
   for (n,&at) in boundaries.iter().enumerate() {
    let _=subtitles::parse_sami(&text[..at]);total+=1;
    if let Some(&next)=boundaries.get(n+1) {
     let mut changed=text[..at].to_owned();changed.push_str(&text[next..]);let _=subtitles::parse_sami(&changed);total+=1;
    }
    for token in ["<",">","'","\"","</BODY>","<P>","<Sync Start=0>"] {
     let mut changed=text[..at].to_owned();changed.push_str(token);changed.push_str(&text[at..]);let _=subtitles::parse_sami(&changed);total+=1;
    }
   }
  }
 }
 println!("{total} bounded authored UTF8 truncation/deletion/markup-insertion probes completed without panic. This audit does not prove arbitrary malformed-caption correctness or total-resource bounds.");
}
