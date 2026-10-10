//! Extract images from PDF

use hayro_syntax::object::MaybeRef::NotRef;
use hayro_syntax::object::MaybeRef::Ref;
use hayro_syntax::object::Object;
use hayro_syntax::object::ObjectIdentifier;
use hayro_syntax::object::dict::keys::HEIGHT;
use hayro_syntax::object::dict::keys::WIDTH;
use hayro_syntax::object::stream::ImageDecodeParams;

fn main() {
    let data = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../hayro-tests/pdfs/custom/image_rgb8.pdf"),
    )
    .unwrap();
    let pdf = hayro_syntax::Pdf::new(data).unwrap();
    let pages = pdf.pages();
    for page in pages.iter() {
        let resources = page.resources();
        let xobjects = &resources.x_objects;
        for (name, maybe_ref) in xobjects.entries() {
            let obj = match maybe_ref {
                Ref(obj_ref) => {
                    let obj = pdf
                        .xref()
                        .get(ObjectIdentifier {
                            obj_number: obj_ref.obj_number,
                            gen_number: obj_ref.gen_number,
                        })
                        .unwrap();
                    obj
                }
                NotRef(b) => b,
            };
            let Object::Stream(s) = obj else {
                continue;
            };
            eprintln!("stream {:?}", s.dict());
            let mut width = None;
            let mut height = None;
            for (stream_name, stream_maybe_ref) in s.dict().entries() {
                let obj = match stream_maybe_ref {
                    Ref(obj_ref) => {
                        let obj = pdf
                            .xref()
                            .get(ObjectIdentifier {
                                obj_number: obj_ref.obj_number,
                                gen_number: obj_ref.gen_number,
                            })
                            .unwrap();
                        obj
                    }
                    NotRef(b) => b,
                };
                eprintln!("name {:?} obj {:?}", stream_name, obj);
                if stream_name.as_ref() == WIDTH
                    && let Object::Number(n) = obj
                {
                    width = Some(n.as_f64() as u32)
                }
                if stream_name.as_ref() == HEIGHT
                    && let Object::Number(n) = obj
                {
                    height = Some(n.as_f64() as u32)
                }
            }
            let Some(width) = width else {
                continue;
            };
            let Some(height) = height else {
                continue;
            };
            let image_params = ImageDecodeParams {
                width,
                height,
                ..Default::default()
            };
            let img = s.decoded_image(&image_params).unwrap();

            eprintln!("image bin {:?}", img.data);
            eprintln!("image data {:?}", img.image_data);
        }
    }
}
