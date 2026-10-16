// تحويل الأسماء العربية إلى حروف لاتينية — لجهاز البصمة الذي لا يدعم العربية.
// يُجرَّب أولاً قاموس الأسماء الشائعة (كلمة بكلمة)، ثم التحويل الحرفي للحروف.

const DICT_RAW: Record<string, string> = {
  // ذكور
  محمد: "Mohammed",
  محمود: "Mahmoud",
  مصطفى: "Mustafa",
  مسلم: "Muslim",
  أحمد: "Ahmad",
  حيدر: "Haider",
  هشام: "Hisham",
  إياد: "Iyad",
  مروان: "Marwan",
  مالك: "Malik",
  منير: "Munir",
  ماجد: "Majed",
  مازن: "Mazen",
  مهند: "Muhannad",
  نبيل: "Nabil",
  نواف: "Nawaf",
  نايف: "Nayef",
  هيثم: "Haitham",
  هاشم: "Hashim",
  وائل: "Wael",
  وليد: "Waleed",
  ياسر: "Yasser",
  يوسف: "Youssef",
  زياد: "Ziad",
  زين: "Zain",
  طارق: "Tarek",
  طلعت: "Talaat",
  عبدالله: "Abdullah",
  عبدالرحمن: "Abdulrahman",
  عبدالعزيز: "Abdulaziz",
  عبداللطيف: "Abdullatif",
  عادل: "Adel",
  عمار: "Ammar",
  عمر: "Omar",
  عثمان: "Othman",
  علي: "Ali",
  أنس: "Anas",
  أيمن: "Ayman",
  بلال: "Bilal",
  بدر: "Badr",
  بشار: "Bashar",
  جمال: "Jamal",
  جبريل: "Jibril",
  حسن: "Hassan",
  حسين: "Hussein",
  حاتم: "Hatem",
  خالد: "Khaled",
  خليل: "Khalil",
  رامي: "Rami",
  راكان: "Rakan",
  رشاد: "Rashad",
  سامي: "Sami",
  سعيد: "Saeed",
  سالم: "Salim",
  سلمان: "Salman",
  سلطان: "Sultan",
  سعود: "Saud",
  سمير: "Samer",
  صلاح: "Salah",
  فادي: "Fadi",
  فارس: "Faris",
  فيصل: "Faisal",
  قاسم: "Qasim",
  كريم: "Karim",
  لؤي: "Loai",
  // إناث
  آمنة: "Amina",
  أمل: "Amal",
  أسماء: "Asma",
  إيمان: "Eman",
  سناء: "Sana",
  وفاء: "Wafaa",
  عبير: "Abir",
  منال: "Manal",
  بشرى: "Bushra",
  تالا: "Tala",
  خديجة: "Khadija",
  دلال: "Dalal",
  دعاء: "Dua",
  ريم: "Reem",
  رنا: "Rana",
  ريهام: "Reham",
  زينب: "Zainob",
  سارة: "Sara",
  سعاد: "Saad",
  سلمى: "Salma",
  شيماء: "Shaimaa",
  عائشة: "Aisha",
  فاطمة: "Fatima",
  لجين: "Lujain",
  لينا: "Lina",
  مريم: "Maryam",
  مها: "Maha",
  نور: "Nour",
  نورهان: "Nourhan",
  هبة: "Heba",
  هند: "Hind",
  ياسمين: "Yasmin",
  الله: "Allah",
};

// إزالة التشكيل وتوحيد الهمزات والألفات لأدق مطابقة في القاموس
const normalize = (w: string) =>
  w
    .replace(/[\u064B-\u0652\u0670]/g, "")
    .replace(/[\u0623\u0625\u0622\u0671]/g, "\u0627")
    .replace(/\u0629/g, "\u0647")
    .replace(/\u0649/g, "\u064A")
    .replace(/\u0624/g, "\u0648")
    .replace(/\u0626/g, "\u064A");

const DICT: Record<string, string> = {};
for (const [k, v] of Object.entries(DICT_RAW)) DICT[normalize(k)] = v;

const MAP: Record<string, string> = {
  "\u0627": "a", "\u0623": "a", "\u0625": "a", "\u0622": "a", "\u0671": "a",
  "\u0621": "", "\u0624": "o", "\u0626": "e",
  "\u0628": "b", "\u062A": "t", "\u062B": "th", "\u062C": "j", "\u062D": "h",
  "\u062E": "kh", "\u062F": "d", "\u0630": "dh", "\u0631": "r", "\u0632": "z",
  "\u0633": "s", "\u0634": "sh", "\u0635": "s", "\u0636": "d", "\u0637": "t",
  "\u0638": "dh", "\u0639": "", "\u063A": "gh",
  "\u0641": "f", "\u0642": "q", "\u0643": "k", "\u0644": "l", "\u0645": "m",
  "\u0646": "n", "\u0647": "h", "\u0629": "a", "\u0649": "a", "\u06CC": "i",
  "\u067E": "p", "\u0686": "ch", "\u0698": "zh", "\u06AF": "g",
};

const hasArabic = (w: string) => /[\u0600-\u06FF]/.test(w);

const translitWord = (w: string): string => {
  // بادئات شائعة في الأسماء العربية
  let prefix = "";
  let rest = w;
  if (rest.startsWith("\u0639\u0628\u062F\u0627\u0644")) {
    prefix = "Abdul";
    rest = rest.slice(5);
  } else if (rest.startsWith("\u0639\u0628\u062F")) {
    prefix = "Abd";
    rest = rest.slice(3);
  } else if (rest.startsWith("\u0627\u0644") && rest.length > 2) {
    prefix = "Al";
    rest = rest.slice(2);
  }
  let out = "";
  for (let i = 0; i < rest.length; i++) {
    const ch = rest[i];
    if (ch === "\u0648") {
      // واو: w في أول الكلمة وu بعدها (وليد→Waleed، فؤاد→Fuad)
      out += i === 0 ? "w" : "u";
      continue;
    }
    if (ch === "\u064A") {
      // ياء: y في أول الكلمة وi بعدها (ياسر→Yasser، سليم→Salim)
      out += i === 0 ? "y" : "i";
      continue;
    }
    out += MAP[ch] ?? ch;
  }
  const s = prefix + out;
  return s.charAt(0).toUpperCase() + s.slice(1);
};

/** يحوّل اسم عربي إلى لاتيني: «محمد أحمد» → "Mohammed Ahmad". */
export function translitArabic(input: string): string {
  return input
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .map((w) => (hasArabic(w) ? DICT[normalize(w)] ?? translitWord(w) : w))
    .join(" ");
}
