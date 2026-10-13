use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;
pub fn id() -> String {
    Uuid::new_v4().to_string()
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Word {
    pub id: String,
    pub english: String,
    pub chinese: String,
    #[serde(default)]
    pub example: String,
    #[serde(default)]
    pub phonetic: String,
}
impl Word {
    pub fn is_pattern(&self) -> bool {
        self.english.split(|c: char| !c.is_ascii_alphabetic()).any(|t| t == "sb" || t == "sth")
        || self.english.contains("one's") || self.english.contains(['/', '.', '(', ')'])
    }
    pub fn spoken(&self) -> &str {
        match self.english.as_str() { "km" => "kilometre", "kg" => "kilogram", "mm" if self.chinese == "毫米" => "millimetre", _ => &self.english }
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Unit {
    pub id: String,
    pub title: String,
    pub words: Vec<Word>,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub edition: String,
    pub grade: String,
    pub volume: String,
    pub source: String,
    pub units: Vec<Unit>,
}
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Library {
    pub revision: u64,
    pub books: Vec<Book>,
    pub history: Vec<Attempt>,
}
impl Library {
    pub fn needs_spelling_review(&self, word_id: &str) -> bool {
        self.history.iter().any(|a| {
            a.word.id == word_id && a.stage == "spell" && a.correct == Some(false)
        })
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Attempt {
    pub id: String,
    pub session: String,
    pub word: Word,
    pub stage: String,
    pub transcript: String,
    pub letters: Option<String>,
    pub correct: Option<bool>,
    pub message: String,
    pub at: String,
    pub source: String,
    #[serde(default)]
    pub recognition: Value,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Quiz {
    pub id: String,
    pub epoch: u64,
    pub title: String,
    pub words: Vec<Word>,
    pub index: usize,
    pub stage: String,
    pub busy: bool,
    pub recording: bool,
    pub feedback: Option<Attempt>,
    pub spelling: Option<Attempt>,
    pub results: Vec<Attempt>,
    pub notice: String,
}
impl Default for Quiz {
    fn default() -> Self {
        Self {
            id: id(),
            epoch: 0,
            title: String::new(),
            words: vec![],
            index: 0,
            stage: "idle".into(),
            busy: false,
            recording: false,
            feedback: None,
            spelling: None,
            results: vec![],
            notice: String::new(),
        }
    }
}
impl Quiz {
    pub fn current(&self) -> Option<&Word> {
        self.words.get(self.index)
    }
    pub fn start(&mut self, title: String, words: Vec<Word>) -> Result<()> {
        if words.is_empty() {
            bail!("请先添加或选择单词")
        }
        let epoch = self.epoch + 1;
        *self = Self {
            epoch,
            title,
            words,
            stage: "spell".into(),
            ..Default::default()
        };
        Ok(())
    }
    pub fn advance(&mut self) -> Result<()> {
        if self.busy || self.recording {
            bail!("请等待当前录音或识别结束")
        }
        match self.stage.as_str() {
            "spell_result" => {
                self.stage = "read".into();
                self.feedback = None;
            }
            "read_result" => self.next(),
            "complete" | "idle" => bail!("请从网页开始一组抽测"),
            _ => bail!("请先长按确认键录音，或选择跳过"),
        };
        self.epoch += 1;
        self.notice.clear();
        Ok(())
    }
    pub fn next(&mut self) {
        self.index += 1;
        self.stage = if self.index >= self.words.len() {
            "complete"
        } else {
            "spell"
        }
        .into();
        self.feedback = None;
        self.spelling = None;
        self.busy = false;
        self.recording = false;
        self.notice.clear();
        self.epoch += 1;
    }
    pub fn accept(&mut self, epoch: u64, transcript: &str, source: &str) -> Option<Attempt> {
        self.accept_recognized(epoch, transcript, source, Value::Null)
    }
    pub fn accept_recognized(&mut self, epoch: u64, transcript: &str, source: &str, recognition: Value) -> Option<Attempt> {
        if epoch != self.epoch || !self.busy {
            return None;
        }
        let word = self.current()?.clone();
        let stage = self.stage.clone();
        let (letters, correct, message) = judge(if stage == "spell" { &word.english } else { word.spoken() }, transcript, &stage);
        let a = Attempt {
            id: id(),
            session: self.id.clone(),
            word,
            stage: stage.clone(),
            transcript: transcript.into(),
            letters,
            correct,
            message,
            at: chrono::Utc::now().to_rfc3339(),
            source: source.into(),
            recognition,
        };
        self.busy = false;
        self.recording = false;
        self.feedback = Some(a.clone());
        // An ASR mismatch is an unconfirmed reading, but still finishes this attempt.
        if correct.is_some() || (stage == "read" && !transcript.trim().is_empty()) {
            self.results.push(a.clone());
            if stage == "spell" {
                self.spelling = Some(a.clone());
                self.stage = "spell_result".into()
            } else {
                self.stage = "read_result".into()
            }
        }
        self.epoch += 1;
        Some(a)
    }
}
// A collapsed word is not evidence of independently spoken letters.
pub fn explicit_letters(text: &str) -> Option<String> {
    let tokens: Vec<_> = text
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|s| !s.is_empty())
        .collect();
    if tokens.len() < 2 || tokens.iter().any(|s| s.len() != 1) {
        return None;
    }
    Some(tokens.concat().to_ascii_uppercase())
}
pub fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphabetic())
        .flat_map(char::to_lowercase)
        .collect()
}
// Exact, reviewed word pairs only. ASR cannot distinguish these by sound;
// this equivalence must never be used to judge separately spoken spelling.
fn reading_homophones_match(expected: &str, recognized: &str) -> bool {
    const PAIRS: &[(&str, &str)] = &[("deer", "dear")];
    fn single_word(text: &str) -> Option<&str> {
        let mut tokens = text.split(|c: char| !c.is_ascii_alphabetic()).filter(|s| !s.is_empty());
        let word = tokens.next()?;
        tokens.next().is_none().then_some(word)
    }
    let (Some(expected), Some(recognized)) = (single_word(expected), single_word(recognized)) else {
        return false;
    };
    PAIRS.iter().any(|&(a, b)| {
        (expected.eq_ignore_ascii_case(a) && recognized.eq_ignore_ascii_case(b))
            || (expected.eq_ignore_ascii_case(b) && recognized.eq_ignore_ascii_case(a))
    })
}
pub fn judge(expected: &str, text: &str, stage: &str) -> (Option<String>, Option<bool>, String) {
    if text.trim().is_empty() {
        return (None, None, "没有听清，请再录一次".into());
    }
    if stage == "spell" {
        match explicit_letters(text) {
            Some(s) => {
                let ok = s.eq_ignore_ascii_case(&normalize(expected));
                let message = if ok {
                    "拼写正确".into()
                } else {
                    format!(
                        "识别到 {}；与答案不一致，请核对录音",
                        s.chars()
                            .map(|c| c.to_string())
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                };
                (Some(s), Some(ok), message)
            }
            None => (
                None,
                None,
                "字母没能分清，请稍放慢，每个字母稍作停顿后重试".into(),
            ),
        }
    } else {
        let standard = |s: &str| normalize(s).replace("millimeter", "millimetre").replace("kilometer", "kilometre");
        let homophone = reading_homophones_match(expected, text);
        let ok = standard(expected) == standard(text) || homophone;
        (
            None,
            ok.then_some(true),
            if homophone {
                format!("识别为同音词 {}，与目标词 {} 同音（不代表发音精度评分）", text.trim(), expected)
            } else if ok {
                "已识别到这个单词（不代表发音精度评分）".into()
            } else {
                "朗读未确认：识别结果与目标词不同，不代表发音错误。可听示范后重试，或继续。".into()
            },
        )
    }
}
pub fn validate_word(w: &mut Word) -> Result<()> {
    w.english = w.english.trim().into();
    w.chinese = w.chinese.trim().into();
    w.example = w.example.trim().into();
    w.phonetic = w.phonetic.trim().into();
    if w.phonetic.chars().count() > 120 || w.phonetic.chars().any(char::is_control) { bail!("音标最多 120 字，不可含控制字符") }
    if w.english.is_empty()
        || w.english.len() > 64
        || !w
            .english
            .bytes()
            .all(|c| c.is_ascii_alphabetic() || b" '-./()!".contains(&c))
        || normalize(&w.english).len() < 2
    {
        bail!("英文须为 2–64 个英文字母，可含空格、撇号、连字符及句型标点")
    }
    if w.chinese.is_empty() || w.chinese.chars().count() > 80 {
        bail!("中文释义须为 1–80 字")
    }
    if w.example.chars().count() > 240 {
        bail!("例句最多 240 字")
    }
    Ok(())
}
/// Short, bounded confirmation sounds generated locally at 16 kHz.
pub fn feedback_samples(correct: bool) -> Vec<i16> {
    let notes: &[(f32, usize)] = if correct { &[(659.25, 130), (880.0, 190)] } else { &[(329.63, 180), (246.94, 220)] };
    let mut samples = vec![0; 160];
    for &(hz, ms) in notes {
        let n = 16 * ms;
        for i in 0..n {
            let edge = (i as f32 / 160.).min((n-1-i) as f32 / 320.).clamp(0., 1.);
            let phase = std::f32::consts::TAU * hz * i as f32 / 16000.;
            let signal = phase.sin() + 0.16 * (2. * phase).sin();
            samples.push((signal * edge * 6500.) as i16);
        }
        samples.extend(std::iter::repeat_n(0, 640));
    }
    samples
}
pub fn wav(samples: &[i16]) -> Vec<u8> {
    let n = (samples.len() * 2) as u32;
    let mut b = vec![];
    b.extend(b"RIFF");
    b.extend((36 + n).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(32000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(n.to_le_bytes());
    for s in samples {
        b.extend(s.to_le_bytes())
    }
    b
}
pub fn pcm(b: &[u8]) -> Result<Vec<i16>> {
    if b.len() < 44 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        bail!("请上传 WAV 音频")
    }
    let mut i = 12;
    let mut format = false;
    let mut data = None;
    while i + 8 <= b.len() {
        let n = u32::from_le_bytes(b[i + 4..i + 8].try_into()?) as usize;
        let start = i + 8;
        let end = start
            .checked_add(n)
            .ok_or_else(|| anyhow::anyhow!("无效 WAV"))?;
        if end > b.len() {
            bail!("WAV 不完整")
        }
        match &b[i..i + 4] {
            b"fmt " => {
                format = n >= 16
                    && b[start..start + 2] == [1, 0]
                    && b[start + 2..start + 4] == [1, 0]
                    && u32::from_le_bytes(b[start + 4..start + 8].try_into()?) == 16000
                    && b[start + 14..start + 16] == [16, 0];
            }
            b"data" => data = Some(&b[start..end]),
            _ => {}
        }
        i = end + (n % 2);
    }
    if !format {
        bail!("需要 16kHz、单声道、16 位 PCM WAV")
    }
    let d = data.ok_or_else(|| anyhow::anyhow!("缺少音频"))?;
    if d.len() % 2 != 0 || d.len() < 6400 || d.len() > 16000 * 2 * 30 {
        bail!("录音长度须为 0.2–30 秒")
    }
    Ok(d.chunks_exact(2)
        .map(|x| i16::from_le_bytes([x[0], x[1]]))
        .collect())
}
pub fn layout(q: &Quiz) -> Value {
    let mut e = vec![
        json!({"type":"rect","x":0,"y":0,"w":399,"h":299}),
        json!({"type":"hline","x":1,"y":40,"w":398}),
        json!({"type":"hline","x":1,"y":267,"w":398}),
    ];
    let mut t = |s: String, x: i32, y: i32, size: i32, width: i32, align: &str| {
        e.push(json!({"type":"text","content":s,"x":x,"y":y,"size":size,"width_limit":width,"align":align,"weight":if size>=24{"bold"}else{"regular"},"font":if s.starts_with('/'){"phonetic"}else{"default"}}));
    };
    t("单词练习".into(), 12, 21, 24, 170, "left");
    t(
        if q.words.is_empty() {
            "本地词库".into()
        } else {
            format!("{} / {}", (q.index + 1).min(q.words.len()), q.words.len())
        },
        386,
        21,
        16,
        150,
        "right",
    );
    if q.stage == "idle" {
        t("今天，练几个单词？".into(), 200, 109, 28, 370, "center");
        t(
            "请在电脑网页选择教材和单元".into(),
            200,
            161,
            18,
            370,
            "center",
        );
        t("拼写与朗读分开练习".into(), 200, 202, 16, 370, "center");
    } else if q.stage == "complete" {
        t("本组练习完成".into(), 200, 102, 32, 370, "center");
        let spell = q.results.iter().filter(|a| a.stage == "spell").count();
        let correct = q
            .results
            .iter()
            .filter(|a| a.stage == "spell" && a.correct == Some(true))
            .count();
        t(
            format!("拼写正确 {correct} / {spell} 次"),
            200,
            158,
            22,
            370,
            "center",
        );
        t(
            "回到网页查看记录或再练一组".into(),
            200,
            211,
            16,
            370,
            "center",
        );
    } else if let Some(w) = q.current() {
        t(q.title.clone(), 12, 58, 12, 376, "left");
        let isspell = q.stage == "spell";
        t(
            if isspell {
                "01  看中文，报字母"
            } else if q.stage == "spell_result" {
                "01  拼写结果"
            } else {
                "02  单独朗读单词"
            }
            .into(),
            200,
            83,
            16,
            370,
            "center",
        );
        if isspell {
            t(w.chinese.clone(), 200, 136, 32, 366, "center");
            t("英文已隐藏".into(), 200, 185, 16, 370, "center");
        } else {
            let size = if w.english.len() <= 18 { 32 } else { 22 };
            if w.english.len() > 30 {
                let split = w.english.char_indices().filter(|(_, c)| *c == ' ').min_by_key(|(i, _)| i.abs_diff(w.english.len()/2)).map(|(i, _)| i);
                if let Some(i) = split {
                    t(w.english[..i].into(), 200, 113, size, 370, "center");
                    t(w.english[i+1..].into(), 200, 139, size, 370, "center");
                } else { t(w.english.clone(), 200, 123, size, 370, "center"); }
            } else { t(w.english.clone(), 200, 116, size, 370, "center"); }
            if !w.phonetic.is_empty() { t(w.phonetic.clone(), 200, 153, 20, 370, "center"); }
            t(w.chinese.clone(), 200, 185, 20, 370, "center");
        }
        let msg = if q.recording {
            "正在录音，松开按键结束".into()
        } else if q.busy {
            "正在本地识别，请稍等".into()
        } else if !q.notice.is_empty() {
            q.notice.clone()
        } else if let Some(a) = &q.feedback {
            if a.stage == "read" {
                if a.correct == Some(true) {
                    if reading_homophones_match(w.spoken(), &a.transcript) {
                        format!("识别为同音词：{}", a.transcript)
                    } else {
                        "已识别到目标词".into()
                    }
                } else if a.transcript.trim().is_empty() {
                    "朗读未确认，请再录一次".into()
                } else {
                    format!("朗读未确认：{}", a.transcript)
                }
            } else if a.correct == Some(true) {
                "拼写正确".into()
            } else if a.correct == Some(false) {
                format!("听到：{}", a.transcript)
            } else {
                "字母未分清，请稍放慢重试".into()
            }
        } else if isspell {
            "例如 A · P · P · L · E".into()
        } else {
            "只读单词，不要再拼字母".into()
        };
        t(msg, 200, 217, 16, 374, "center");
    }
    let footer = match q.stage.as_str() {
        "spell" | "read" => "长按确认录音 · 下键跳过",
        "spell_result" => "确认键继续朗读 · 上键听示范",
        "read_result" => "确认键下一词 · 上键听示范",
        _ => "词库与记录保存在这台电脑",
    };
    t(footer.into(), 200, 283, 16, 382, "center");
    json!({"canvas":{"w":400,"h":300},"elements":e})
}
#[cfg(test)]
mod tests {
    use super::*;
    fn word() -> Word {
        Word {
            id: id(),
            english: "apple".into(),
            chinese: "苹果".into(),
            example: "".into(),
            phonetic: "/ˈæpl/".into(),
        }
    }
    #[test]
    fn ipa_is_hidden_until_answer_and_old_words_load() {
        let old = r#"{"id":"x","english":"apple","chinese":"苹果","example":""}"#;
        assert_eq!(serde_json::from_str::<Word>(old).unwrap().phonetic, "");
        let mut q = Quiz::default(); q.start("test".into(),vec![word()]).unwrap();
        assert!(!layout(&q).to_string().contains("ˈæpl"));
        for stage in ["spell_result", "read", "read_result"] {
            q.stage = stage.into(); assert!(layout(&q).to_string().contains("ˈæpl"));
        }
    }
    #[test]
    fn conservative_spelling() {
        assert_eq!(judge("apple", "A P L E.", "spell").1, Some(false));
        assert_eq!(judge("apple", "A P P L E.", "spell").1, Some(true));
        assert_eq!(judge("friend", "Friend.", "spell").1, None);
        assert_eq!(judge("friend", "F R E I N D.", "spell").1, Some(false));
        assert_eq!(
            judge("because", "B, e, c, a, u, s, e.", "spell").1,
            Some(true)
        );
        assert_eq!(judge("apple", "A P P L E apple", "spell").1, None);
    }
    #[test]
    fn phases_and_stale_jobs() {
        let mut q = Quiz::default();
        q.start("demo".into(), vec![word()]).unwrap();
        q.busy = true;
        let e = q.epoch;
        q.accept(e, "apple", "test").unwrap();
        assert_eq!(q.stage, "spell");
        q.busy = true;
        let e = q.epoch;
        q.accept(e, "A P P L E", "test").unwrap();
        assert_eq!(q.stage, "spell_result");
        q.advance().unwrap();
        assert_eq!(q.stage, "read");
        q.busy = true;
        let e = q.epoch;
        q.next();
        assert!(q.accept(e, "apple", "test").is_none());
        assert_eq!(q.stage, "complete");
    }
    #[test]
    fn uncertain_reading_finishes_without_a_pronunciation_error() {
        let mut q = Quiz::default();
        let mut deer = word();
        deer.english = "deer".into();
        deer.chinese = "鹿".into();
        q.start("demo".into(), vec![deer]).unwrap();
        q.busy = true;
        q.accept(q.epoch, "D E E R", "test").unwrap();
        q.advance().unwrap();
        q.busy = true;
        let attempt = q.accept(q.epoch, "Yeah.", "test").unwrap();
        assert_eq!(attempt.correct, None);
        assert_eq!(attempt.transcript, "Yeah.");
        assert!(attempt.message.contains("朗读未确认"));
        assert_eq!(q.stage, "read_result");
        assert_eq!(q.results.len(), 2);
        assert_eq!(q.spelling.as_ref().unwrap().correct, Some(true));
        let screen = layout(&q).to_string();
        assert!(screen.contains("朗读未确认：Yeah."));
        assert!(screen.contains("确认键下一词 · 上键听示范"));
        assert!(!screen.contains("字母未分清"));
        q.advance().unwrap();
        assert_eq!(q.stage, "complete");
    }
    #[test]
    fn matched_reading_is_recognized_and_empty_reading_can_retry() {
        let mut q = Quiz::default();
        q.start("demo".into(), vec![word()]).unwrap();
        q.stage = "read".into();
        q.busy = true;
        let empty = q.accept(q.epoch, "  ", "test").unwrap();
        assert_eq!(empty.correct, None);
        assert_eq!(q.stage, "read");
        assert!(q.results.is_empty());
        let screen = layout(&q).to_string();
        assert!(screen.contains("朗读未确认，请再录一次"));
        assert!(!screen.contains("字母未分清"));
        q.busy = true;
        let matched = q.accept(q.epoch, "Apple.", "test").unwrap();
        assert_eq!(matched.correct, Some(true));
        assert_eq!(q.stage, "read_result");
        assert!(matched.message.contains("不代表发音精度评分"));
        assert!(layout(&q).to_string().contains("已识别到目标词"));
    }
    #[test]
    fn deer_and_dear_match_only_as_reading_homophones() {
        let result = judge("deer", "Dear.", "read");
        assert_eq!(result.1, Some(true));
        assert!(result.2.contains("识别为同音词 Dear."));
        assert!(result.2.contains("不代表发音精度评分"));
        assert_eq!(judge("dear", "Deer.", "read").1, Some(true));
        assert_eq!(judge("deer", "Yeah.", "read").1, None);
        assert_eq!(judge("deer", "D.", "read").1, None);
        assert_eq!(judge("deer", "D E A R", "read").1, None);
        assert_eq!(judge("deer", "D. E. A. R.", "read").1, None);
        assert_eq!(judge("deer", "D E A R", "spell").1, Some(false));
        assert_eq!(judge("deer", "Dear.", "spell").1, None);
        assert_eq!(judge("deer", "D E E R", "spell").1, Some(true));
        assert_eq!(judge("beer", "Dear.", "read").1, None);
        assert_eq!(judge("deer", "Dear friend.", "read").1, None);
        let mut q = Quiz::default();
        let mut deer = word();
        deer.english = "deer".into();
        deer.chinese = "鹿".into();
        q.start("demo".into(), vec![deer]).unwrap();
        q.stage = "read".into();
        q.busy = true;
        q.accept(q.epoch, "Dear.", "test").unwrap();
        assert_eq!(q.stage, "read_result");
        assert!(layout(&q).to_string().contains("识别为同音词：Dear."));
    }
    #[test]
    fn unresolved_spelling_stays_in_spelling() {
        let mut q = Quiz::default();
        q.start("demo".into(), vec![word()]).unwrap();
        q.busy = true;
        let attempt = q.accept(q.epoch, "Apple.", "test").unwrap();
        assert_eq!(attempt.correct, None);
        assert_eq!(q.stage, "spell");
        assert!(q.results.is_empty());
        assert!(q.spelling.is_none());
        assert!(q.advance().is_err());
    }
    #[test]
    fn wrong_only_uses_spelling_errors_including_legacy_history() {
        let mut q = Quiz::default();
        let w = word();
        q.start("demo".into(), vec![w.clone()]).unwrap();
        q.stage = "read".into();
        q.busy = true;
        let mut reading = q.accept(q.epoch, "Yeah.", "test").unwrap();
        let mut library = Library::default();
        library.history.push(reading.clone());
        assert!(!library.needs_spelling_review(&w.id));
        // Existing records used false for ASR mismatches; they must also be excluded.
        reading.correct = Some(false);
        library.history.push(reading);
        assert!(!library.needs_spelling_review(&w.id));
        q.stage = "spell".into();
        q.busy = true;
        library.history.push(q.accept(q.epoch, "A P P L E", "test").unwrap());
        assert!(!library.needs_spelling_review(&w.id));
        q.stage = "spell".into();
        q.busy = true;
        library.history.push(q.accept(q.epoch, "A P L E", "test").unwrap());
        assert!(library.needs_spelling_review(&w.id));
        assert!(!library.needs_spelling_review("another-word"));
    }
    #[test]
    fn feedback_is_distinct_bounded_and_playable() {
        let yes=feedback_samples(true); let no=feedback_samples(false);
        assert_ne!(yes,no);
        for samples in [yes,no] {
            assert!(samples.len() < 16000); assert!(samples.iter().all(|s| s.unsigned_abs()<8000));
            assert_eq!(samples[0],0); assert_eq!(samples.last(),Some(&0));
            assert_eq!(pcm(&wav(&samples)).unwrap(),samples);
        }
    }
    #[test]
    fn wav_validation() {
        let b = wav(&vec![42; 16000]);
        assert_eq!(pcm(&b).unwrap(), vec![42; 16000]);
        let mut bad = b.clone();
        bad[24] = 1;
        assert!(pcm(&bad).is_err());
        assert!(pcm(&b[..b.len() - 1]).is_err());
        assert!(pcm(&wav(&[0; 10])).is_err());
    }
}
