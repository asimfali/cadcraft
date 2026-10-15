//! Data from a drawing file that CADCraft does not model, kept so that saving the drawing writes
//! it back: other programs' objects, extension dictionaries, xdata and properties (#528).
//!
//! It is kept as the file's own DXF groups, with the file's handles. The file reader fills it and
//! the writer puts it back, pointing references at whatever now has the referenced role (the
//! named object dictionary, a table record, a block, an entity); references to objects that are
//! gone are dropped. Commands don't edit it.

use std::collections::BTreeMap;

use crate::{Handle, RawTag};

/// What a file handle stood for when the object it named is one CADCraft writes itself (under a
/// new handle on save): a kind and an upper-case name, e.g. `("LAYER", "WALLS")`,
/// `("TABLE", "LAYER")`, `("BLOCK", "DOOR")`, `("NOD", "")` for the named object dictionary and
/// `("NOD", "ACAD_GROUP")` for one of its entries.
pub type ForeignKey = (String, String);

/// A non-graphical object of a type or role CADCraft does not model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ForeignObject {
    /// Its handle in the file it came from (kept on save).
    pub handle: Handle,
    /// Its DXF type (group 0), e.g. `XRECORD`.
    pub kind: String,
    /// Its groups after the type, as read.
    pub tags: Vec<RawTag>,
}

/// What an entity, table record or block carries beyond what CADCraft models.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ForeignExtra {
    /// Its extension dictionary (one of [`Foreign::objects`]).
    pub xdict: Option<Handle>,
    /// Objects of [`Foreign::objects`] that react to it (persistent reactors).
    pub reactors: Vec<Handle>,
    /// Other applications' extended data of a table record or block: groups from `1001` on. An
    /// entity's is part of the entity ([`crate::Common::xdata`]), so it follows the entity's copies.
    pub xdata: Vec<RawTag>,
    /// The colour book colour (group 430) and the true colour (group 420) it was read with: it is
    /// written back while the colour is unchanged.
    pub color_book: Option<(String, String)>,
    /// Other unmodelled property groups (material 347, shadow mode 284).
    pub props: Vec<RawTag>,
}

impl ForeignExtra {
    pub fn is_empty(&self) -> bool {
        self.xdict.is_none() && self.reactors.is_empty() && self.xdata.is_empty() && self.color_book.is_none() && self.props.is_empty()
    }
}

/// Everything of a file CADCraft keeps without modelling it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Foreign {
    /// CLASS records (groups after `0 CLASS`) of the kept objects' and entities' types.
    pub classes: Vec<Vec<RawTag>>,
    /// Kept objects, in file order.
    pub objects: Vec<ForeignObject>,
    /// Entries of the named object dictionary that CADCraft doesn't write itself: (key, object).
    pub dictionary: Vec<(String, Handle)>,
    /// Extras of entities, by handle.
    pub entities: BTreeMap<Handle, ForeignExtra>,
    /// Extras of table records (`("LAYER", "WALLS")`), tables (`("TABLE", "LAYER")`) and blocks
    /// (`("BLOCK", "DOOR")`), by [`ForeignKey`].
    pub records: BTreeMap<ForeignKey, ForeignExtra>,
    /// The roles of the file handles the kept data refers to, for objects CADCraft rewrites.
    pub targets: BTreeMap<Handle, ForeignKey>,
    /// The file's data store section (ACDSDATA, AutoCAD 2013 and later), as read: its groups
    /// after the section name. It holds the ACIS geometry of 3D solids, regions, bodies and
    /// surfaces, which from 2013 on those entities don't carry themselves: each `ACDSRECORD`
    /// names its entity with a 320 handle (see [`Foreign::data_store_entities`]).
    pub data_store: Vec<RawTag>,
}

impl Foreign {
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
            && self.dictionary.is_empty()
            && self.entities.is_empty()
            && self.records.is_empty()
            && self.classes.is_empty()
            && self.targets.is_empty()
            && self.data_store.is_empty()
    }

    /// The entities the data store has records for (their 320 handles).
    pub fn data_store_entities(&self) -> std::collections::BTreeSet<Handle> {
        self.data_store.iter().filter(|t| t.code == 320).filter_map(|t| Handle::parse_hex(t.value.trim())).collect()
    }

    /// The kept objects by handle, for looking many of them up.
    pub fn object_index(&self) -> std::collections::HashMap<Handle, &ForeignObject> {
        self.objects.iter().map(|o| (o.handle, o)).collect()
    }

    /// The XRECORDs in the extension dictionary of `entity`: (key, data groups), in dictionary
    /// order. `index` is [`Foreign::object_index`].
    pub fn xrecords(&self, index: &std::collections::HashMap<Handle, &ForeignObject>, entity: Handle) -> Vec<(String, Vec<RawTag>)> {
        let Some(dict) = self.entities.get(&entity).and_then(|x| x.xdict).and_then(|h| index.get(&h)) else { return Vec::new() };
        let mut out = Vec::new();
        let mut key: Option<&str> = None;
        for t in dict.tags.iter().take(MAX_DICT_GROUPS) {
            match t.code {
                3 => key = Some(t.value.as_str()),
                350 | 360 => {
                    let rec = key.take().zip(Handle::parse_hex(&t.value)).and_then(|(k, h)| Some((k, index.get(&h)?)));
                    if let Some((k, o)) = rec.filter(|(_, o)| o.kind == "XRECORD") {
                        out.push((k.to_string(), o.xrecord_data().to_vec()));
                    }
                }
                _ => {}
            }
        }
        out
    }
}

impl Foreign {
    /// Set the XRECORD `key` in the extension dictionary of `entity` to text `value` (one group
    /// 1000, as wire-list add-ins write them), making the dictionary and the record when they are
    /// missing; `handles` gives the handles of new objects (two at most are taken).
    pub fn set_xrecord_text(&mut self, entity: Handle, key: &str, value: &str, handles: [Handle; 2]) {
        let tag = |code: i32, value: String| RawTag { code, value };
        let owned = |h: Handle, owner: Handle, class: &str| {
            vec![
                tag(5, h.hex()),
                tag(102, "{ACAD_REACTORS".into()),
                tag(330, owner.hex()),
                tag(102, "}".into()),
                tag(330, owner.hex()),
                tag(100, class.into()),
            ]
        };
        let value: String = value.chars().take(MAX_XRECORD_TEXT).collect();
        // The dictionary (made when missing).
        let existing =
            self.entities.get(&entity).and_then(|x| x.xdict).filter(|d| self.objects.iter().any(|o| o.handle == *d && o.kind == "DICTIONARY"));
        let made = existing.is_none();
        let dict = match existing {
            Some(d) => d,
            None => {
                let d = handles[0];
                let mut tags = owned(d, entity, "AcDbDictionary");
                tags.extend([tag(280, "1".into()), tag(281, "1".into())]);
                self.objects.push(ForeignObject { handle: d, kind: "DICTIONARY".into(), tags });
                self.entities.entry(entity).or_default().xdict = Some(d);
                d
            }
        };
        // Its entry `key`, and the record it names.
        let rec = self.objects.iter().find(|o| o.handle == dict).and_then(|o| {
            let mut it = o.tags.iter();
            while let Some(t) = it.next() {
                if t.code == 3 && t.value.eq_ignore_ascii_case(key) {
                    return it.next().filter(|n| matches!(n.code, 350 | 360)).and_then(|n| Handle::parse_hex(&n.value));
                }
            }
            None
        });
        if let Some(o) = rec.and_then(|r| self.objects.iter_mut().find(|o| o.handle == r && o.kind == "XRECORD")) {
            let keep = o.tags.iter().position(|t| t.code == 100 && t.value.trim().eq_ignore_ascii_case("AcDbXrecord")).map_or(0, |i| i + 1);
            let keep = keep + usize::from(o.tags.get(keep).is_some_and(|t| t.code == 280));
            o.tags.truncate(keep);
            o.tags.push(tag(1000, value));
            return;
        }
        // The first handle went to a dictionary made just now.
        let r = if made { handles[1] } else { handles[0] };
        let mut tags = owned(r, dict, "AcDbXrecord");
        tags.extend([tag(280, "1".into()), tag(1000, value)]);
        self.objects.push(ForeignObject { handle: r, kind: "XRECORD".into(), tags });
        if let Some(o) = self.objects.iter_mut().find(|o| o.handle == dict) {
            o.tags.extend([tag(3, key.to_string()), tag(350, r.hex())]);
        }
    }
}

/// Longest text an XRECORD set here holds.
const MAX_XRECORD_TEXT: usize = 4096;

/// Most groups of a dictionary read when looking for its entries.
const MAX_DICT_GROUPS: usize = 100_000;

impl ForeignObject {
    /// An XRECORD's data groups: those after its `100 AcDbXrecord` subclass marker and the
    /// duplicate record cloning flag (280) that follows it.
    pub fn xrecord_data(&self) -> &[RawTag] {
        let Some(i) = self.tags.iter().position(|t| t.code == 100 && t.value.trim().eq_ignore_ascii_case("AcDbXrecord")) else { return &[] };
        let rest = self.tags.get(i + 1..).unwrap_or(&[]);
        match rest.first() {
            Some(t) if t.code == 280 => rest.get(1..).unwrap_or(&[]),
            _ => rest,
        }
    }
}
