/* Offline Unicode palette. Names follow UnicodeData via Python unicodedata 15.1.0.
 * Combining marks are stored literally: the display-only dotted circle is never inserted.
 */
(function () {
'use strict';
const layouts = [["hebrew","Hebrew / Aramaic (square script)",true,[["Letters",[["א","HEBREW LETTER ALEF"],["ב","HEBREW LETTER BET"],["ג","HEBREW LETTER GIMEL"],["ד","HEBREW LETTER DALET"],["ה","HEBREW LETTER HE"],["ו","HEBREW LETTER VAV"],["ז","HEBREW LETTER ZAYIN"],["ח","HEBREW LETTER HET"],["ט","HEBREW LETTER TET"],["י","HEBREW LETTER YOD"],["ך","HEBREW LETTER FINAL KAF"],["כ","HEBREW LETTER KAF"],["ל","HEBREW LETTER LAMED"],["ם","HEBREW LETTER FINAL MEM"],["מ","HEBREW LETTER MEM"],["ן","HEBREW LETTER FINAL NUN"],["נ","HEBREW LETTER NUN"],["ס","HEBREW LETTER SAMEKH"],["ע","HEBREW LETTER AYIN"],["ף","HEBREW LETTER FINAL PE"],["פ","HEBREW LETTER PE"],["ץ","HEBREW LETTER FINAL TSADI"],["צ","HEBREW LETTER TSADI"],["ק","HEBREW LETTER QOF"],["ר","HEBREW LETTER RESH"],["ש","HEBREW LETTER SHIN"],["ת","HEBREW LETTER TAV"]]],["Vowel points",[["ְ","HEBREW POINT SHEVA"],["ֱ","HEBREW POINT HATAF SEGOL"],["ֲ","HEBREW POINT HATAF PATAH"],["ֳ","HEBREW POINT HATAF QAMATS"],["ִ","HEBREW POINT HIRIQ"],["ֵ","HEBREW POINT TSERE"],["ֶ","HEBREW POINT SEGOL"],["ַ","HEBREW POINT PATAH"],["ָ","HEBREW POINT QAMATS"],["ֹ","HEBREW POINT HOLAM"],["ֺ","HEBREW POINT HOLAM HASER FOR VAV"],["ֻ","HEBREW POINT QUBUTS"],["ּ","HEBREW POINT DAGESH OR MAPIQ"],["ֽ","HEBREW POINT METEG"],["ֿ","HEBREW POINT RAFE"],["ׁ","HEBREW POINT SHIN DOT"],["ׂ","HEBREW POINT SIN DOT"],["ׄ","HEBREW MARK UPPER DOT"],["ׅ","HEBREW MARK LOWER DOT"],["ׇ","HEBREW POINT QAMATS QATAN"]]],["Punctuation",[["־","HEBREW PUNCTUATION MAQAF"],["׀","HEBREW PUNCTUATION PASEQ"],["׃","HEBREW PUNCTUATION SOF PASUQ"],["׆","HEBREW PUNCTUATION NUN HAFUKHA"],["׳","HEBREW PUNCTUATION GERESH"],["״","HEBREW PUNCTUATION GERSHAYIM"]]],["Cantillation",[["֑","HEBREW ACCENT ETNAHTA"],["֒","HEBREW ACCENT SEGOL"],["֓","HEBREW ACCENT SHALSHELET"],["֔","HEBREW ACCENT ZAQEF QATAN"],["֕","HEBREW ACCENT ZAQEF GADOL"],["֖","HEBREW ACCENT TIPEHA"],["֗","HEBREW ACCENT REVIA"],["֘","HEBREW ACCENT ZARQA"],["֙","HEBREW ACCENT PASHTA"],["֚","HEBREW ACCENT YETIV"],["֛","HEBREW ACCENT TEVIR"],["֜","HEBREW ACCENT GERESH"],["֝","HEBREW ACCENT GERESH MUQDAM"],["֞","HEBREW ACCENT GERSHAYIM"],["֟","HEBREW ACCENT QARNEY PARA"],["֠","HEBREW ACCENT TELISHA GEDOLA"],["֡","HEBREW ACCENT PAZER"],["֢","HEBREW ACCENT ATNAH HAFUKH"],["֣","HEBREW ACCENT MUNAH"],["֤","HEBREW ACCENT MAHAPAKH"],["֥","HEBREW ACCENT MERKHA"],["֦","HEBREW ACCENT MERKHA KEFULA"],["֧","HEBREW ACCENT DARGA"],["֨","HEBREW ACCENT QADMA"],["֩","HEBREW ACCENT TELISHA QETANA"],["֪","HEBREW ACCENT YERAH BEN YOMO"],["֫","HEBREW ACCENT OLE"],["֬","HEBREW ACCENT ILUY"],["֭","HEBREW ACCENT DEHI"],["֮","HEBREW ACCENT ZINOR"],["֯","HEBREW MARK MASORA CIRCLE"]]]]],["arabic","Arabic / Persian",true,[["Letters",[["ء","ARABIC LETTER HAMZA"],["آ","ARABIC LETTER ALEF WITH MADDA ABOVE"],["أ","ARABIC LETTER ALEF WITH HAMZA ABOVE"],["ؤ","ARABIC LETTER WAW WITH HAMZA ABOVE"],["إ","ARABIC LETTER ALEF WITH HAMZA BELOW"],["ئ","ARABIC LETTER YEH WITH HAMZA ABOVE"],["ا","ARABIC LETTER ALEF"],["ب","ARABIC LETTER BEH"],["ة","ARABIC LETTER TEH MARBUTA"],["ت","ARABIC LETTER TEH"],["ث","ARABIC LETTER THEH"],["ج","ARABIC LETTER JEEM"],["ح","ARABIC LETTER HAH"],["خ","ARABIC LETTER KHAH"],["د","ARABIC LETTER DAL"],["ذ","ARABIC LETTER THAL"],["ر","ARABIC LETTER REH"],["ز","ARABIC LETTER ZAIN"],["س","ARABIC LETTER SEEN"],["ش","ARABIC LETTER SHEEN"],["ص","ARABIC LETTER SAD"],["ض","ARABIC LETTER DAD"],["ط","ARABIC LETTER TAH"],["ظ","ARABIC LETTER ZAH"],["ع","ARABIC LETTER AIN"],["غ","ARABIC LETTER GHAIN"],["ف","ARABIC LETTER FEH"],["ق","ARABIC LETTER QAF"],["ك","ARABIC LETTER KAF"],["ل","ARABIC LETTER LAM"],["م","ARABIC LETTER MEEM"],["ن","ARABIC LETTER NOON"],["ه","ARABIC LETTER HEH"],["و","ARABIC LETTER WAW"],["ى","ARABIC LETTER ALEF MAKSURA"],["ي","ARABIC LETTER YEH"],["ٱ","ARABIC LETTER ALEF WASLA"],["پ","ARABIC LETTER PEH"],["چ","ARABIC LETTER TCHEH"],["ژ","ARABIC LETTER JEH"],["ک","ARABIC LETTER KEHEH"],["گ","ARABIC LETTER GAF"],["ی","ARABIC LETTER FARSI YEH"]]],["Vowel points",[["ً","ARABIC FATHATAN"],["ٌ","ARABIC DAMMATAN"],["ٍ","ARABIC KASRATAN"],["َ","ARABIC FATHA"],["ُ","ARABIC DAMMA"],["ِ","ARABIC KASRA"],["ّ","ARABIC SHADDA"],["ْ","ARABIC SUKUN"],["ٓ","ARABIC MADDAH ABOVE"],["ٔ","ARABIC HAMZA ABOVE"],["ٕ","ARABIC HAMZA BELOW"],["ٖ","ARABIC SUBSCRIPT ALEF"],["ٗ","ARABIC INVERTED DAMMA"],["٘","ARABIC MARK NOON GHUNNA"],["ٙ","ARABIC ZWARAKAY"],["ٚ","ARABIC VOWEL SIGN SMALL V ABOVE"],["ٛ","ARABIC VOWEL SIGN INVERTED SMALL V ABOVE"],["ٜ","ARABIC VOWEL SIGN DOT BELOW"],["ٝ","ARABIC REVERSED DAMMA"],["ٞ","ARABIC FATHA WITH TWO DOTS"],["ٟ","ARABIC WAVY HAMZA BELOW"],["ٰ","ARABIC LETTER SUPERSCRIPT ALEF"]]],["Punctuation and digits",[["،","ARABIC COMMA"],["؛","ARABIC SEMICOLON"],["؟","ARABIC QUESTION MARK"],["ـ","ARABIC TATWEEL"],["٠","ARABIC-INDIC DIGIT ZERO"],["١","ARABIC-INDIC DIGIT ONE"],["٢","ARABIC-INDIC DIGIT TWO"],["٣","ARABIC-INDIC DIGIT THREE"],["٤","ARABIC-INDIC DIGIT FOUR"],["٥","ARABIC-INDIC DIGIT FIVE"],["٦","ARABIC-INDIC DIGIT SIX"],["٧","ARABIC-INDIC DIGIT SEVEN"],["٨","ARABIC-INDIC DIGIT EIGHT"],["٩","ARABIC-INDIC DIGIT NINE"],["۰","EXTENDED ARABIC-INDIC DIGIT ZERO"],["۱","EXTENDED ARABIC-INDIC DIGIT ONE"],["۲","EXTENDED ARABIC-INDIC DIGIT TWO"],["۳","EXTENDED ARABIC-INDIC DIGIT THREE"],["۴","EXTENDED ARABIC-INDIC DIGIT FOUR"],["۵","EXTENDED ARABIC-INDIC DIGIT FIVE"],["۶","EXTENDED ARABIC-INDIC DIGIT SIX"],["۷","EXTENDED ARABIC-INDIC DIGIT SEVEN"],["۸","EXTENDED ARABIC-INDIC DIGIT EIGHT"],["۹","EXTENDED ARABIC-INDIC DIGIT NINE"]]]]],["syriac","Syriac",true,[["Letters",[["ܐ","SYRIAC LETTER ALAPH"],["ܑ","SYRIAC LETTER SUPERSCRIPT ALAPH"],["ܒ","SYRIAC LETTER BETH"],["ܓ","SYRIAC LETTER GAMAL"],["ܔ","SYRIAC LETTER GAMAL GARSHUNI"],["ܕ","SYRIAC LETTER DALATH"],["ܖ","SYRIAC LETTER DOTLESS DALATH RISH"],["ܗ","SYRIAC LETTER HE"],["ܘ","SYRIAC LETTER WAW"],["ܙ","SYRIAC LETTER ZAIN"],["ܚ","SYRIAC LETTER HETH"],["ܛ","SYRIAC LETTER TETH"],["ܜ","SYRIAC LETTER TETH GARSHUNI"],["ܝ","SYRIAC LETTER YUDH"],["ܞ","SYRIAC LETTER YUDH HE"],["ܟ","SYRIAC LETTER KAPH"],["ܠ","SYRIAC LETTER LAMADH"],["ܡ","SYRIAC LETTER MIM"],["ܢ","SYRIAC LETTER NUN"],["ܣ","SYRIAC LETTER SEMKATH"],["ܤ","SYRIAC LETTER FINAL SEMKATH"],["ܥ","SYRIAC LETTER E"],["ܦ","SYRIAC LETTER PE"],["ܧ","SYRIAC LETTER REVERSED PE"],["ܨ","SYRIAC LETTER SADHE"],["ܩ","SYRIAC LETTER QAPH"],["ܪ","SYRIAC LETTER RISH"],["ܫ","SYRIAC LETTER SHIN"],["ܬ","SYRIAC LETTER TAW"],["ܭ","SYRIAC LETTER PERSIAN BHETH"],["ܮ","SYRIAC LETTER PERSIAN GHAMAL"],["ܯ","SYRIAC LETTER PERSIAN DHALATH"]]],["Vowel points",[["ܰ","SYRIAC PTHAHA ABOVE"],["ܱ","SYRIAC PTHAHA BELOW"],["ܲ","SYRIAC PTHAHA DOTTED"],["ܳ","SYRIAC ZQAPHA ABOVE"],["ܴ","SYRIAC ZQAPHA BELOW"],["ܵ","SYRIAC ZQAPHA DOTTED"],["ܶ","SYRIAC RBASA ABOVE"],["ܷ","SYRIAC RBASA BELOW"],["ܸ","SYRIAC DOTTED ZLAMA HORIZONTAL"],["ܹ","SYRIAC DOTTED ZLAMA ANGULAR"],["ܺ","SYRIAC HBASA ABOVE"],["ܻ","SYRIAC HBASA BELOW"],["ܼ","SYRIAC HBASA-ESASA DOTTED"],["ܽ","SYRIAC ESASA ABOVE"],["ܾ","SYRIAC ESASA BELOW"],["ܿ","SYRIAC RWAHA"],["݀","SYRIAC FEMININE DOT"],["݁","SYRIAC QUSHSHAYA"],["݂","SYRIAC RUKKAKHA"],["݃","SYRIAC TWO VERTICAL DOTS ABOVE"],["݄","SYRIAC TWO VERTICAL DOTS BELOW"],["݅","SYRIAC THREE DOTS ABOVE"],["݆","SYRIAC THREE DOTS BELOW"],["݇","SYRIAC OBLIQUE LINE ABOVE"],["݈","SYRIAC OBLIQUE LINE BELOW"],["݉","SYRIAC MUSIC"],["݊","SYRIAC BARREKH"]]],["Punctuation",[["܀","SYRIAC END OF PARAGRAPH"],["܁","SYRIAC SUPRALINEAR FULL STOP"],["܂","SYRIAC SUBLINEAR FULL STOP"],["܃","SYRIAC SUPRALINEAR COLON"],["܄","SYRIAC SUBLINEAR COLON"],["܅","SYRIAC HORIZONTAL COLON"],["܆","SYRIAC COLON SKEWED LEFT"],["܇","SYRIAC COLON SKEWED RIGHT"],["܈","SYRIAC SUPRALINEAR COLON SKEWED LEFT"],["܉","SYRIAC SUBLINEAR COLON SKEWED RIGHT"],["܊","SYRIAC CONTRACTION"]]]]],["greek","Greek (polytonic)",false,[["Lowercase",[["α","GREEK SMALL LETTER ALPHA"],["β","GREEK SMALL LETTER BETA"],["γ","GREEK SMALL LETTER GAMMA"],["δ","GREEK SMALL LETTER DELTA"],["ε","GREEK SMALL LETTER EPSILON"],["ζ","GREEK SMALL LETTER ZETA"],["η","GREEK SMALL LETTER ETA"],["θ","GREEK SMALL LETTER THETA"],["ι","GREEK SMALL LETTER IOTA"],["κ","GREEK SMALL LETTER KAPPA"],["λ","GREEK SMALL LETTER LAMDA"],["μ","GREEK SMALL LETTER MU"],["ν","GREEK SMALL LETTER NU"],["ξ","GREEK SMALL LETTER XI"],["ο","GREEK SMALL LETTER OMICRON"],["π","GREEK SMALL LETTER PI"],["ρ","GREEK SMALL LETTER RHO"],["ς","GREEK SMALL LETTER FINAL SIGMA"],["σ","GREEK SMALL LETTER SIGMA"],["τ","GREEK SMALL LETTER TAU"],["υ","GREEK SMALL LETTER UPSILON"],["φ","GREEK SMALL LETTER PHI"],["χ","GREEK SMALL LETTER CHI"],["ψ","GREEK SMALL LETTER PSI"],["ω","GREEK SMALL LETTER OMEGA"]]],["Uppercase",[["Α","GREEK CAPITAL LETTER ALPHA"],["Β","GREEK CAPITAL LETTER BETA"],["Γ","GREEK CAPITAL LETTER GAMMA"],["Δ","GREEK CAPITAL LETTER DELTA"],["Ε","GREEK CAPITAL LETTER EPSILON"],["Ζ","GREEK CAPITAL LETTER ZETA"],["Η","GREEK CAPITAL LETTER ETA"],["Θ","GREEK CAPITAL LETTER THETA"],["Ι","GREEK CAPITAL LETTER IOTA"],["Κ","GREEK CAPITAL LETTER KAPPA"],["Λ","GREEK CAPITAL LETTER LAMDA"],["Μ","GREEK CAPITAL LETTER MU"],["Ν","GREEK CAPITAL LETTER NU"],["Ξ","GREEK CAPITAL LETTER XI"],["Ο","GREEK CAPITAL LETTER OMICRON"],["Π","GREEK CAPITAL LETTER PI"],["Ρ","GREEK CAPITAL LETTER RHO"],["Σ","GREEK CAPITAL LETTER SIGMA"],["Τ","GREEK CAPITAL LETTER TAU"],["Υ","GREEK CAPITAL LETTER UPSILON"],["Φ","GREEK CAPITAL LETTER PHI"],["Χ","GREEK CAPITAL LETTER CHI"],["Ψ","GREEK CAPITAL LETTER PSI"],["Ω","GREEK CAPITAL LETTER OMEGA"]]],["Accents and breathings",[["̀","COMBINING GRAVE ACCENT"],["́","COMBINING ACUTE ACCENT"],["̄","COMBINING MACRON"],["̆","COMBINING BREVE"],["̈","COMBINING DIAERESIS"],["̓","COMBINING COMMA ABOVE"],["̔","COMBINING REVERSED COMMA ABOVE"],["͂","COMBINING GREEK PERISPOMENI"],["ͅ","COMBINING GREEK YPOGEGRAMMENI"]]],["Archaic letters and punctuation",[["ϐ","GREEK BETA SYMBOL"],["ϑ","GREEK THETA SYMBOL"],["ϕ","GREEK PHI SYMBOL"],["ϖ","GREEK PI SYMBOL"],["Ϙ","GREEK LETTER ARCHAIC KOPPA"],["ϙ","GREEK SMALL LETTER ARCHAIC KOPPA"],["Ϛ","GREEK LETTER STIGMA"],["ϛ","GREEK SMALL LETTER STIGMA"],["Ϝ","GREEK LETTER DIGAMMA"],["ϝ","GREEK SMALL LETTER DIGAMMA"],["Ϟ","GREEK LETTER KOPPA"],["ϟ","GREEK SMALL LETTER KOPPA"],["Ϡ","GREEK LETTER SAMPI"],["ϡ","GREEK SMALL LETTER SAMPI"],["ϰ","GREEK KAPPA SYMBOL"],["ϱ","GREEK RHO SYMBOL"],["ϲ","GREEK LUNATE SIGMA SYMBOL"],["ϵ","GREEK LUNATE EPSILON SYMBOL"],["ʹ","GREEK NUMERAL SIGN"],["͵","GREEK LOWER NUMERAL SIGN"],[";","GREEK QUESTION MARK"],["·","GREEK ANO TELEIA"]]]]],["ethiopic","Ethiopic (Geʿez)",false,[["Syllables ሀ–ቿ",[["ሀ","ETHIOPIC SYLLABLE HA"],["ሁ","ETHIOPIC SYLLABLE HU"],["ሂ","ETHIOPIC SYLLABLE HI"],["ሃ","ETHIOPIC SYLLABLE HAA"],["ሄ","ETHIOPIC SYLLABLE HEE"],["ህ","ETHIOPIC SYLLABLE HE"],["ሆ","ETHIOPIC SYLLABLE HO"],["ሇ","ETHIOPIC SYLLABLE HOA"],["ለ","ETHIOPIC SYLLABLE LA"],["ሉ","ETHIOPIC SYLLABLE LU"],["ሊ","ETHIOPIC SYLLABLE LI"],["ላ","ETHIOPIC SYLLABLE LAA"],["ሌ","ETHIOPIC SYLLABLE LEE"],["ል","ETHIOPIC SYLLABLE LE"],["ሎ","ETHIOPIC SYLLABLE LO"],["ሏ","ETHIOPIC SYLLABLE LWA"],["ሐ","ETHIOPIC SYLLABLE HHA"],["ሑ","ETHIOPIC SYLLABLE HHU"],["ሒ","ETHIOPIC SYLLABLE HHI"],["ሓ","ETHIOPIC SYLLABLE HHAA"],["ሔ","ETHIOPIC SYLLABLE HHEE"],["ሕ","ETHIOPIC SYLLABLE HHE"],["ሖ","ETHIOPIC SYLLABLE HHO"],["ሗ","ETHIOPIC SYLLABLE HHWA"],["መ","ETHIOPIC SYLLABLE MA"],["ሙ","ETHIOPIC SYLLABLE MU"],["ሚ","ETHIOPIC SYLLABLE MI"],["ማ","ETHIOPIC SYLLABLE MAA"],["ሜ","ETHIOPIC SYLLABLE MEE"],["ም","ETHIOPIC SYLLABLE ME"],["ሞ","ETHIOPIC SYLLABLE MO"],["ሟ","ETHIOPIC SYLLABLE MWA"],["ሠ","ETHIOPIC SYLLABLE SZA"],["ሡ","ETHIOPIC SYLLABLE SZU"],["ሢ","ETHIOPIC SYLLABLE SZI"],["ሣ","ETHIOPIC SYLLABLE SZAA"],["ሤ","ETHIOPIC SYLLABLE SZEE"],["ሥ","ETHIOPIC SYLLABLE SZE"],["ሦ","ETHIOPIC SYLLABLE SZO"],["ሧ","ETHIOPIC SYLLABLE SZWA"],["ረ","ETHIOPIC SYLLABLE RA"],["ሩ","ETHIOPIC SYLLABLE RU"],["ሪ","ETHIOPIC SYLLABLE RI"],["ራ","ETHIOPIC SYLLABLE RAA"],["ሬ","ETHIOPIC SYLLABLE REE"],["ር","ETHIOPIC SYLLABLE RE"],["ሮ","ETHIOPIC SYLLABLE RO"],["ሯ","ETHIOPIC SYLLABLE RWA"],["ሰ","ETHIOPIC SYLLABLE SA"],["ሱ","ETHIOPIC SYLLABLE SU"],["ሲ","ETHIOPIC SYLLABLE SI"],["ሳ","ETHIOPIC SYLLABLE SAA"],["ሴ","ETHIOPIC SYLLABLE SEE"],["ስ","ETHIOPIC SYLLABLE SE"],["ሶ","ETHIOPIC SYLLABLE SO"],["ሷ","ETHIOPIC SYLLABLE SWA"],["ሸ","ETHIOPIC SYLLABLE SHA"],["ሹ","ETHIOPIC SYLLABLE SHU"],["ሺ","ETHIOPIC SYLLABLE SHI"],["ሻ","ETHIOPIC SYLLABLE SHAA"],["ሼ","ETHIOPIC SYLLABLE SHEE"],["ሽ","ETHIOPIC SYLLABLE SHE"],["ሾ","ETHIOPIC SYLLABLE SHO"],["ሿ","ETHIOPIC SYLLABLE SHWA"],["ቀ","ETHIOPIC SYLLABLE QA"],["ቁ","ETHIOPIC SYLLABLE QU"],["ቂ","ETHIOPIC SYLLABLE QI"],["ቃ","ETHIOPIC SYLLABLE QAA"],["ቄ","ETHIOPIC SYLLABLE QEE"],["ቅ","ETHIOPIC SYLLABLE QE"],["ቆ","ETHIOPIC SYLLABLE QO"],["ቇ","ETHIOPIC SYLLABLE QOA"],["ቈ","ETHIOPIC SYLLABLE QWA"],["ቊ","ETHIOPIC SYLLABLE QWI"],["ቋ","ETHIOPIC SYLLABLE QWAA"],["ቌ","ETHIOPIC SYLLABLE QWEE"],["ቍ","ETHIOPIC SYLLABLE QWE"],["ቐ","ETHIOPIC SYLLABLE QHA"],["ቑ","ETHIOPIC SYLLABLE QHU"],["ቒ","ETHIOPIC SYLLABLE QHI"],["ቓ","ETHIOPIC SYLLABLE QHAA"],["ቔ","ETHIOPIC SYLLABLE QHEE"],["ቕ","ETHIOPIC SYLLABLE QHE"],["ቖ","ETHIOPIC SYLLABLE QHO"],["ቘ","ETHIOPIC SYLLABLE QHWA"],["ቚ","ETHIOPIC SYLLABLE QHWI"],["ቛ","ETHIOPIC SYLLABLE QHWAA"],["ቜ","ETHIOPIC SYLLABLE QHWEE"],["ቝ","ETHIOPIC SYLLABLE QHWE"],["በ","ETHIOPIC SYLLABLE BA"],["ቡ","ETHIOPIC SYLLABLE BU"],["ቢ","ETHIOPIC SYLLABLE BI"],["ባ","ETHIOPIC SYLLABLE BAA"],["ቤ","ETHIOPIC SYLLABLE BEE"],["ብ","ETHIOPIC SYLLABLE BE"],["ቦ","ETHIOPIC SYLLABLE BO"],["ቧ","ETHIOPIC SYLLABLE BWA"],["ቨ","ETHIOPIC SYLLABLE VA"],["ቩ","ETHIOPIC SYLLABLE VU"],["ቪ","ETHIOPIC SYLLABLE VI"],["ቫ","ETHIOPIC SYLLABLE VAA"],["ቬ","ETHIOPIC SYLLABLE VEE"],["ቭ","ETHIOPIC SYLLABLE VE"],["ቮ","ETHIOPIC SYLLABLE VO"],["ቯ","ETHIOPIC SYLLABLE VWA"],["ተ","ETHIOPIC SYLLABLE TA"],["ቱ","ETHIOPIC SYLLABLE TU"],["ቲ","ETHIOPIC SYLLABLE TI"],["ታ","ETHIOPIC SYLLABLE TAA"],["ቴ","ETHIOPIC SYLLABLE TEE"],["ት","ETHIOPIC SYLLABLE TE"],["ቶ","ETHIOPIC SYLLABLE TO"],["ቷ","ETHIOPIC SYLLABLE TWA"],["ቸ","ETHIOPIC SYLLABLE CA"],["ቹ","ETHIOPIC SYLLABLE CU"],["ቺ","ETHIOPIC SYLLABLE CI"],["ቻ","ETHIOPIC SYLLABLE CAA"],["ቼ","ETHIOPIC SYLLABLE CEE"],["ች","ETHIOPIC SYLLABLE CE"],["ቾ","ETHIOPIC SYLLABLE CO"],["ቿ","ETHIOPIC SYLLABLE CWA"]]],["Syllables ኀ–ዿ",[["ኀ","ETHIOPIC SYLLABLE XA"],["ኁ","ETHIOPIC SYLLABLE XU"],["ኂ","ETHIOPIC SYLLABLE XI"],["ኃ","ETHIOPIC SYLLABLE XAA"],["ኄ","ETHIOPIC SYLLABLE XEE"],["ኅ","ETHIOPIC SYLLABLE XE"],["ኆ","ETHIOPIC SYLLABLE XO"],["ኇ","ETHIOPIC SYLLABLE XOA"],["ኈ","ETHIOPIC SYLLABLE XWA"],["ኊ","ETHIOPIC SYLLABLE XWI"],["ኋ","ETHIOPIC SYLLABLE XWAA"],["ኌ","ETHIOPIC SYLLABLE XWEE"],["ኍ","ETHIOPIC SYLLABLE XWE"],["ነ","ETHIOPIC SYLLABLE NA"],["ኑ","ETHIOPIC SYLLABLE NU"],["ኒ","ETHIOPIC SYLLABLE NI"],["ና","ETHIOPIC SYLLABLE NAA"],["ኔ","ETHIOPIC SYLLABLE NEE"],["ን","ETHIOPIC SYLLABLE NE"],["ኖ","ETHIOPIC SYLLABLE NO"],["ኗ","ETHIOPIC SYLLABLE NWA"],["ኘ","ETHIOPIC SYLLABLE NYA"],["ኙ","ETHIOPIC SYLLABLE NYU"],["ኚ","ETHIOPIC SYLLABLE NYI"],["ኛ","ETHIOPIC SYLLABLE NYAA"],["ኜ","ETHIOPIC SYLLABLE NYEE"],["ኝ","ETHIOPIC SYLLABLE NYE"],["ኞ","ETHIOPIC SYLLABLE NYO"],["ኟ","ETHIOPIC SYLLABLE NYWA"],["አ","ETHIOPIC SYLLABLE GLOTTAL A"],["ኡ","ETHIOPIC SYLLABLE GLOTTAL U"],["ኢ","ETHIOPIC SYLLABLE GLOTTAL I"],["ኣ","ETHIOPIC SYLLABLE GLOTTAL AA"],["ኤ","ETHIOPIC SYLLABLE GLOTTAL EE"],["እ","ETHIOPIC SYLLABLE GLOTTAL E"],["ኦ","ETHIOPIC SYLLABLE GLOTTAL O"],["ኧ","ETHIOPIC SYLLABLE GLOTTAL WA"],["ከ","ETHIOPIC SYLLABLE KA"],["ኩ","ETHIOPIC SYLLABLE KU"],["ኪ","ETHIOPIC SYLLABLE KI"],["ካ","ETHIOPIC SYLLABLE KAA"],["ኬ","ETHIOPIC SYLLABLE KEE"],["ክ","ETHIOPIC SYLLABLE KE"],["ኮ","ETHIOPIC SYLLABLE KO"],["ኯ","ETHIOPIC SYLLABLE KOA"],["ኰ","ETHIOPIC SYLLABLE KWA"],["ኲ","ETHIOPIC SYLLABLE KWI"],["ኳ","ETHIOPIC SYLLABLE KWAA"],["ኴ","ETHIOPIC SYLLABLE KWEE"],["ኵ","ETHIOPIC SYLLABLE KWE"],["ኸ","ETHIOPIC SYLLABLE KXA"],["ኹ","ETHIOPIC SYLLABLE KXU"],["ኺ","ETHIOPIC SYLLABLE KXI"],["ኻ","ETHIOPIC SYLLABLE KXAA"],["ኼ","ETHIOPIC SYLLABLE KXEE"],["ኽ","ETHIOPIC SYLLABLE KXE"],["ኾ","ETHIOPIC SYLLABLE KXO"],["ዀ","ETHIOPIC SYLLABLE KXWA"],["ዂ","ETHIOPIC SYLLABLE KXWI"],["ዃ","ETHIOPIC SYLLABLE KXWAA"],["ዄ","ETHIOPIC SYLLABLE KXWEE"],["ዅ","ETHIOPIC SYLLABLE KXWE"],["ወ","ETHIOPIC SYLLABLE WA"],["ዉ","ETHIOPIC SYLLABLE WU"],["ዊ","ETHIOPIC SYLLABLE WI"],["ዋ","ETHIOPIC SYLLABLE WAA"],["ዌ","ETHIOPIC SYLLABLE WEE"],["ው","ETHIOPIC SYLLABLE WE"],["ዎ","ETHIOPIC SYLLABLE WO"],["ዏ","ETHIOPIC SYLLABLE WOA"],["ዐ","ETHIOPIC SYLLABLE PHARYNGEAL A"],["ዑ","ETHIOPIC SYLLABLE PHARYNGEAL U"],["ዒ","ETHIOPIC SYLLABLE PHARYNGEAL I"],["ዓ","ETHIOPIC SYLLABLE PHARYNGEAL AA"],["ዔ","ETHIOPIC SYLLABLE PHARYNGEAL EE"],["ዕ","ETHIOPIC SYLLABLE PHARYNGEAL E"],["ዖ","ETHIOPIC SYLLABLE PHARYNGEAL O"],["ዘ","ETHIOPIC SYLLABLE ZA"],["ዙ","ETHIOPIC SYLLABLE ZU"],["ዚ","ETHIOPIC SYLLABLE ZI"],["ዛ","ETHIOPIC SYLLABLE ZAA"],["ዜ","ETHIOPIC SYLLABLE ZEE"],["ዝ","ETHIOPIC SYLLABLE ZE"],["ዞ","ETHIOPIC SYLLABLE ZO"],["ዟ","ETHIOPIC SYLLABLE ZWA"],["ዠ","ETHIOPIC SYLLABLE ZHA"],["ዡ","ETHIOPIC SYLLABLE ZHU"],["ዢ","ETHIOPIC SYLLABLE ZHI"],["ዣ","ETHIOPIC SYLLABLE ZHAA"],["ዤ","ETHIOPIC SYLLABLE ZHEE"],["ዥ","ETHIOPIC SYLLABLE ZHE"],["ዦ","ETHIOPIC SYLLABLE ZHO"],["ዧ","ETHIOPIC SYLLABLE ZHWA"],["የ","ETHIOPIC SYLLABLE YA"],["ዩ","ETHIOPIC SYLLABLE YU"],["ዪ","ETHIOPIC SYLLABLE YI"],["ያ","ETHIOPIC SYLLABLE YAA"],["ዬ","ETHIOPIC SYLLABLE YEE"],["ይ","ETHIOPIC SYLLABLE YE"],["ዮ","ETHIOPIC SYLLABLE YO"],["ዯ","ETHIOPIC SYLLABLE YOA"],["ደ","ETHIOPIC SYLLABLE DA"],["ዱ","ETHIOPIC SYLLABLE DU"],["ዲ","ETHIOPIC SYLLABLE DI"],["ዳ","ETHIOPIC SYLLABLE DAA"],["ዴ","ETHIOPIC SYLLABLE DEE"],["ድ","ETHIOPIC SYLLABLE DE"],["ዶ","ETHIOPIC SYLLABLE DO"],["ዷ","ETHIOPIC SYLLABLE DWA"],["ዸ","ETHIOPIC SYLLABLE DDA"],["ዹ","ETHIOPIC SYLLABLE DDU"],["ዺ","ETHIOPIC SYLLABLE DDI"],["ዻ","ETHIOPIC SYLLABLE DDAA"],["ዼ","ETHIOPIC SYLLABLE DDEE"],["ዽ","ETHIOPIC SYLLABLE DDE"],["ዾ","ETHIOPIC SYLLABLE DDO"],["ዿ","ETHIOPIC SYLLABLE DDWA"]]],["Syllables ጀ–ፚ",[["ጀ","ETHIOPIC SYLLABLE JA"],["ጁ","ETHIOPIC SYLLABLE JU"],["ጂ","ETHIOPIC SYLLABLE JI"],["ጃ","ETHIOPIC SYLLABLE JAA"],["ጄ","ETHIOPIC SYLLABLE JEE"],["ጅ","ETHIOPIC SYLLABLE JE"],["ጆ","ETHIOPIC SYLLABLE JO"],["ጇ","ETHIOPIC SYLLABLE JWA"],["ገ","ETHIOPIC SYLLABLE GA"],["ጉ","ETHIOPIC SYLLABLE GU"],["ጊ","ETHIOPIC SYLLABLE GI"],["ጋ","ETHIOPIC SYLLABLE GAA"],["ጌ","ETHIOPIC SYLLABLE GEE"],["ግ","ETHIOPIC SYLLABLE GE"],["ጎ","ETHIOPIC SYLLABLE GO"],["ጏ","ETHIOPIC SYLLABLE GOA"],["ጐ","ETHIOPIC SYLLABLE GWA"],["ጒ","ETHIOPIC SYLLABLE GWI"],["ጓ","ETHIOPIC SYLLABLE GWAA"],["ጔ","ETHIOPIC SYLLABLE GWEE"],["ጕ","ETHIOPIC SYLLABLE GWE"],["ጘ","ETHIOPIC SYLLABLE GGA"],["ጙ","ETHIOPIC SYLLABLE GGU"],["ጚ","ETHIOPIC SYLLABLE GGI"],["ጛ","ETHIOPIC SYLLABLE GGAA"],["ጜ","ETHIOPIC SYLLABLE GGEE"],["ጝ","ETHIOPIC SYLLABLE GGE"],["ጞ","ETHIOPIC SYLLABLE GGO"],["ጟ","ETHIOPIC SYLLABLE GGWAA"],["ጠ","ETHIOPIC SYLLABLE THA"],["ጡ","ETHIOPIC SYLLABLE THU"],["ጢ","ETHIOPIC SYLLABLE THI"],["ጣ","ETHIOPIC SYLLABLE THAA"],["ጤ","ETHIOPIC SYLLABLE THEE"],["ጥ","ETHIOPIC SYLLABLE THE"],["ጦ","ETHIOPIC SYLLABLE THO"],["ጧ","ETHIOPIC SYLLABLE THWA"],["ጨ","ETHIOPIC SYLLABLE CHA"],["ጩ","ETHIOPIC SYLLABLE CHU"],["ጪ","ETHIOPIC SYLLABLE CHI"],["ጫ","ETHIOPIC SYLLABLE CHAA"],["ጬ","ETHIOPIC SYLLABLE CHEE"],["ጭ","ETHIOPIC SYLLABLE CHE"],["ጮ","ETHIOPIC SYLLABLE CHO"],["ጯ","ETHIOPIC SYLLABLE CHWA"],["ጰ","ETHIOPIC SYLLABLE PHA"],["ጱ","ETHIOPIC SYLLABLE PHU"],["ጲ","ETHIOPIC SYLLABLE PHI"],["ጳ","ETHIOPIC SYLLABLE PHAA"],["ጴ","ETHIOPIC SYLLABLE PHEE"],["ጵ","ETHIOPIC SYLLABLE PHE"],["ጶ","ETHIOPIC SYLLABLE PHO"],["ጷ","ETHIOPIC SYLLABLE PHWA"],["ጸ","ETHIOPIC SYLLABLE TSA"],["ጹ","ETHIOPIC SYLLABLE TSU"],["ጺ","ETHIOPIC SYLLABLE TSI"],["ጻ","ETHIOPIC SYLLABLE TSAA"],["ጼ","ETHIOPIC SYLLABLE TSEE"],["ጽ","ETHIOPIC SYLLABLE TSE"],["ጾ","ETHIOPIC SYLLABLE TSO"],["ጿ","ETHIOPIC SYLLABLE TSWA"],["ፀ","ETHIOPIC SYLLABLE TZA"],["ፁ","ETHIOPIC SYLLABLE TZU"],["ፂ","ETHIOPIC SYLLABLE TZI"],["ፃ","ETHIOPIC SYLLABLE TZAA"],["ፄ","ETHIOPIC SYLLABLE TZEE"],["ፅ","ETHIOPIC SYLLABLE TZE"],["ፆ","ETHIOPIC SYLLABLE TZO"],["ፇ","ETHIOPIC SYLLABLE TZOA"],["ፈ","ETHIOPIC SYLLABLE FA"],["ፉ","ETHIOPIC SYLLABLE FU"],["ፊ","ETHIOPIC SYLLABLE FI"],["ፋ","ETHIOPIC SYLLABLE FAA"],["ፌ","ETHIOPIC SYLLABLE FEE"],["ፍ","ETHIOPIC SYLLABLE FE"],["ፎ","ETHIOPIC SYLLABLE FO"],["ፏ","ETHIOPIC SYLLABLE FWA"],["ፐ","ETHIOPIC SYLLABLE PA"],["ፑ","ETHIOPIC SYLLABLE PU"],["ፒ","ETHIOPIC SYLLABLE PI"],["ፓ","ETHIOPIC SYLLABLE PAA"],["ፔ","ETHIOPIC SYLLABLE PEE"],["ፕ","ETHIOPIC SYLLABLE PE"],["ፖ","ETHIOPIC SYLLABLE PO"],["ፗ","ETHIOPIC SYLLABLE PWA"],["ፘ","ETHIOPIC SYLLABLE RYA"],["ፙ","ETHIOPIC SYLLABLE MYA"],["ፚ","ETHIOPIC SYLLABLE FYA"]]],["Marks, punctuation and numbers",[["፝","ETHIOPIC COMBINING GEMINATION AND VOWEL LENGTH MARK"],["፞","ETHIOPIC COMBINING VOWEL LENGTH MARK"],["፟","ETHIOPIC COMBINING GEMINATION MARK"],["፠","ETHIOPIC SECTION MARK"],["፡","ETHIOPIC WORDSPACE"],["።","ETHIOPIC FULL STOP"],["፣","ETHIOPIC COMMA"],["፤","ETHIOPIC SEMICOLON"],["፥","ETHIOPIC COLON"],["፦","ETHIOPIC PREFACE COLON"],["፧","ETHIOPIC QUESTION MARK"],["፨","ETHIOPIC PARAGRAPH SEPARATOR"],["፩","ETHIOPIC DIGIT ONE"],["፪","ETHIOPIC DIGIT TWO"],["፫","ETHIOPIC DIGIT THREE"],["፬","ETHIOPIC DIGIT FOUR"],["፭","ETHIOPIC DIGIT FIVE"],["፮","ETHIOPIC DIGIT SIX"],["፯","ETHIOPIC DIGIT SEVEN"],["፰","ETHIOPIC DIGIT EIGHT"],["፱","ETHIOPIC DIGIT NINE"],["፲","ETHIOPIC NUMBER TEN"],["፳","ETHIOPIC NUMBER TWENTY"],["፴","ETHIOPIC NUMBER THIRTY"],["፵","ETHIOPIC NUMBER FORTY"],["፶","ETHIOPIC NUMBER FIFTY"],["፷","ETHIOPIC NUMBER SIXTY"],["፸","ETHIOPIC NUMBER SEVENTY"],["፹","ETHIOPIC NUMBER EIGHTY"],["፺","ETHIOPIC NUMBER NINETY"],["፻","ETHIOPIC NUMBER HUNDRED"],["፼","ETHIOPIC NUMBER TEN THOUSAND"]]]]],["phoenician","Phoenician",true,[["Letters",[["𐤀","PHOENICIAN LETTER ALF"],["𐤁","PHOENICIAN LETTER BET"],["𐤂","PHOENICIAN LETTER GAML"],["𐤃","PHOENICIAN LETTER DELT"],["𐤄","PHOENICIAN LETTER HE"],["𐤅","PHOENICIAN LETTER WAU"],["𐤆","PHOENICIAN LETTER ZAI"],["𐤇","PHOENICIAN LETTER HET"],["𐤈","PHOENICIAN LETTER TET"],["𐤉","PHOENICIAN LETTER YOD"],["𐤊","PHOENICIAN LETTER KAF"],["𐤋","PHOENICIAN LETTER LAMD"],["𐤌","PHOENICIAN LETTER MEM"],["𐤍","PHOENICIAN LETTER NUN"],["𐤎","PHOENICIAN LETTER SEMK"],["𐤏","PHOENICIAN LETTER AIN"],["𐤐","PHOENICIAN LETTER PE"],["𐤑","PHOENICIAN LETTER SADE"],["𐤒","PHOENICIAN LETTER QOF"],["𐤓","PHOENICIAN LETTER ROSH"],["𐤔","PHOENICIAN LETTER SHIN"],["𐤕","PHOENICIAN LETTER TAU"]]],["Numbers and separator",[["𐤖","PHOENICIAN NUMBER ONE"],["𐤗","PHOENICIAN NUMBER TEN"],["𐤘","PHOENICIAN NUMBER TWENTY"],["𐤙","PHOENICIAN NUMBER ONE HUNDRED"],["𐤚","PHOENICIAN NUMBER TWO"],["𐤛","PHOENICIAN NUMBER THREE"],["𐤟","PHOENICIAN WORD SEPARATOR"]]]]],["latin","Latin / transliteration",false,[["Letters",[["A","LATIN CAPITAL LETTER A"],["B","LATIN CAPITAL LETTER B"],["C","LATIN CAPITAL LETTER C"],["D","LATIN CAPITAL LETTER D"],["E","LATIN CAPITAL LETTER E"],["F","LATIN CAPITAL LETTER F"],["G","LATIN CAPITAL LETTER G"],["H","LATIN CAPITAL LETTER H"],["I","LATIN CAPITAL LETTER I"],["J","LATIN CAPITAL LETTER J"],["K","LATIN CAPITAL LETTER K"],["L","LATIN CAPITAL LETTER L"],["M","LATIN CAPITAL LETTER M"],["N","LATIN CAPITAL LETTER N"],["O","LATIN CAPITAL LETTER O"],["P","LATIN CAPITAL LETTER P"],["Q","LATIN CAPITAL LETTER Q"],["R","LATIN CAPITAL LETTER R"],["S","LATIN CAPITAL LETTER S"],["T","LATIN CAPITAL LETTER T"],["U","LATIN CAPITAL LETTER U"],["V","LATIN CAPITAL LETTER V"],["W","LATIN CAPITAL LETTER W"],["X","LATIN CAPITAL LETTER X"],["Y","LATIN CAPITAL LETTER Y"],["Z","LATIN CAPITAL LETTER Z"],["a","LATIN SMALL LETTER A"],["b","LATIN SMALL LETTER B"],["c","LATIN SMALL LETTER C"],["d","LATIN SMALL LETTER D"],["e","LATIN SMALL LETTER E"],["f","LATIN SMALL LETTER F"],["g","LATIN SMALL LETTER G"],["h","LATIN SMALL LETTER H"],["i","LATIN SMALL LETTER I"],["j","LATIN SMALL LETTER J"],["k","LATIN SMALL LETTER K"],["l","LATIN SMALL LETTER L"],["m","LATIN SMALL LETTER M"],["n","LATIN SMALL LETTER N"],["o","LATIN SMALL LETTER O"],["p","LATIN SMALL LETTER P"],["q","LATIN SMALL LETTER Q"],["r","LATIN SMALL LETTER R"],["s","LATIN SMALL LETTER S"],["t","LATIN SMALL LETTER T"],["u","LATIN SMALL LETTER U"],["v","LATIN SMALL LETTER V"],["w","LATIN SMALL LETTER W"],["x","LATIN SMALL LETTER X"],["y","LATIN SMALL LETTER Y"],["z","LATIN SMALL LETTER Z"],["æ","LATIN SMALL LETTER AE"],["Æ","LATIN CAPITAL LETTER AE"],["œ","LATIN SMALL LIGATURE OE"],["Œ","LATIN CAPITAL LIGATURE OE"],["ø","LATIN SMALL LETTER O WITH STROKE"],["Ø","LATIN CAPITAL LETTER O WITH STROKE"],["ı","LATIN SMALL LETTER DOTLESS I"],["ſ","LATIN SMALL LETTER LONG S"],["ə","LATIN SMALL LETTER SCHWA"],["ɛ","LATIN SMALL LETTER OPEN E"],["ʒ","LATIN SMALL LETTER EZH"],["ʾ","MODIFIER LETTER RIGHT HALF RING"],["ʿ","MODIFIER LETTER LEFT HALF RING"]]],["Combining marks",[["̀","COMBINING GRAVE ACCENT"],["́","COMBINING ACUTE ACCENT"],["̂","COMBINING CIRCUMFLEX ACCENT"],["̃","COMBINING TILDE"],["̄","COMBINING MACRON"],["̆","COMBINING BREVE"],["̇","COMBINING DOT ABOVE"],["̈","COMBINING DIAERESIS"],["̊","COMBINING RING ABOVE"],["̌","COMBINING CARON"],["̣","COMBINING DOT BELOW"],["̤","COMBINING DIAERESIS BELOW"],["̥","COMBINING RING BELOW"],["̧","COMBINING CEDILLA"],["̨","COMBINING OGONEK"],["̭","COMBINING CIRCUMFLEX ACCENT BELOW"],["̮","COMBINING BREVE BELOW"],["̱","COMBINING MACRON BELOW"]]],["Punctuation",[["§","SECTION SIGN"],["·","MIDDLE DOT"],["‐","HYPHEN"],["–","EN DASH"],["—","EM DASH"],["‘","LEFT SINGLE QUOTATION MARK"],["’","RIGHT SINGLE QUOTATION MARK"],["“","LEFT DOUBLE QUOTATION MARK"],["”","RIGHT DOUBLE QUOTATION MARK"],["†","DAGGER"],["‡","DOUBLE DAGGER"],["…","HORIZONTAL ELLIPSIS"],["′","PRIME"],["″","DOUBLE PRIME"]]]]]];
const isMark = text => /^\p{M}+$/u.test(text);
const codePoint = text => 'U+' + text.codePointAt(0).toString(16).toUpperCase().padStart(4, '0');

// Ethiopic's seven orders occupy slots 0–6 in each eight-code-point series.
// Preserve holes in labialized series; slot 7 is not a uniform vowel order.
// Reference: https://www.unicode.org/charts/PDF/U1200.pdf
function ethiopicRows(keys) {
    const rows = new Map();
    const extras = [];
    for (const key of keys) {
        const cp = key[0].codePointAt(0);
        if (cp < 0x1200 || cp > 0x1357) { extras.push(key); continue; }
        const base = cp - cp % 8;
        if (!rows.has(base)) rows.set(base, Array(8).fill(null));
        rows.get(base)[cp % 8] = key;
    }
    return { rows: [...rows.values()], extras };
}

// Textarea offsets are UTF-16. Expand a selection that falls inside a surrogate pair.
function selection(value, start, end) {
    const inside = offset => offset > 0 && offset < value.length &&
        /[\uD800-\uDBFF]/.test(value[offset - 1]) && /[\uDC00-\uDFFF]/.test(value[offset]);
    if (start === end && inside(start)) start = end = start + 1;
    else { if (inside(start)) start--; if (inside(end)) end++; }
    return [start, end];
}
function edit(value, start, end, inserted) {
    [start, end] = selection(value, start, end);
    if (isMark(inserted)) {
        if (start !== end) {
            if (!/^\p{L}\p{M}*$/u.test(value.slice(start, end))) {
                throw Error('Select one letter with its points, or place the cursor just after a letter.');
            }
            start = end;
        }
        if (!/\p{L}\p{M}*$/u.test(value.slice(0, start))) {
            throw Error('Place the cursor just after a letter to add a point or accent.');
        }
    }
    return { value: value.slice(0, start) + inserted + value.slice(end), cursor: start + inserted.length };
}
function backspace(value, start, end) {
    [start, end] = selection(value, start, end);
    if (start === end) start -= [...value.slice(0, start)].at(-1)?.length || 0;
    return { value: value.slice(0, start) + value.slice(end), cursor: start };
}
function fromCodePoint(input) {
    const hex = input.trim().replace(/^U\+/i, '');
    if (!/^[0-9a-f]{4,6}$/i.test(hex)) throw Error('Enter one code point, for example U+05B0.');
    const n = parseInt(hex, 16);
    if (n > 0x10ffff || (n >= 0xd800 && n <= 0xdfff)) throw Error('This is not a Unicode scalar value.');
    const text = String.fromCodePoint(n);
    if (text !== ' ' && !/^[\p{L}\p{M}\p{N}\p{P}\p{S}]$/u.test(text)) {
        throw Error('Choose a letter, mark, number, punctuation mark or symbol.');
    }
    return text;
}
// Pure editing functions are also exercised without a browser.
if (typeof module !== 'undefined') module.exports = { edit, backspace, fromCodePoint, layouts, ethiopicRows };
if (typeof document === 'undefined') return;
const el = id => document.getElementById(id);
const defaultTarget = () => document.querySelector('[data-run-text]') || el('text');
let target = defaultTarget();
let history = [];
const status = (text, error = false) => {
    el('keyboardStatus').textContent = text;
    el('keyboardStatus').classList.toggle('error', error);
};
const updateTarget = () => { el('keyboardTarget').textContent = target.id === 'comment' ? 'Typing into: Review notes' : target.hasAttribute('data-run-text') ? `Typing into: Run ${Number(target.dataset.runText)+1} (${target.dataset.languageLabel})` : 'Typing into: Your transcription'; };
document.addEventListener('focusin', event => {
    if (event.target.matches('[data-run-text], #text, #comment')) { target = event.target; updateTarget(); }
});
document.addEventListener('input', event => {
    if (event.isTrusted && event.target.matches('[data-run-text], #text, #comment')) history = [];
});
function apply(operation) {
    if (el('save').disabled) return;
    try {
        const before = { value: target.value, start: target.selectionStart, end: target.selectionEnd };
        const after = operation(before.value, before.start, before.end);
        history.push({ field: target, before, after });
        target.value = after.value;
        target.focus({ preventScroll: true });
        target.setSelectionRange(after.cursor, after.cursor);
        target.dispatchEvent(new Event('input', { bubbles: true }));
        status(target.id === 'comment' ? 'Review notes updated.' : 'Transcription updated.');
    } catch (error) { status(error.message, true); }
}
function insert(text) { apply((value, start, end) => edit(value, start, end, text)); }
function bindButton(button, action) {
    button.type = 'button';
    // Mouse/touch users keep their editing caret; keyboard users can tab to every key.
    button.addEventListener('pointerdown', event => event.preventDefault());
    button.addEventListener('click', action);
}
function renderKeys() {
    const layout = layouts.find(layout => layout[0] === el('keyboardLanguage').value);
    const groupIndex = Number(el('keyboardGroup').value);
    const query = el('keyboardSearch').value.trim().toUpperCase();
    const ethiopic = layout[0] === 'ethiopic';
    const groups = query || ethiopic ? layout[3] : [layout[3][groupIndex]];
    const keys = groups.flatMap(group => group[1]).filter(([text, name]) =>
        !query || name.includes(query) || text === el('keyboardSearch').value.trim() || codePoint(text).includes(query));
    el('keyboardKeys').dir = layout[2] ? 'rtl' : 'ltr';
    const makeKey = ([text, name]) => {
        const button = document.createElement('button');
        button.textContent = (isMark(text) ? '◌' : '') + text;
        button.title = name + ' · ' + codePoint(text);
        button.setAttribute('aria-label', button.title);
        button.dataset.codepoint = codePoint(text);
        bindButton(button, () => insert(text));
        return button;
    };
    el('keyboardKeys').classList.toggle('ethiopic-scroll', ethiopic && !query);
    if (ethiopic && !query) {
        const table = document.createElement('table');
        table.className = 'ethiopic-table';
        const caption = table.createCaption();
        caption.textContent = 'Base letters × vowel forms';
        const header = table.createTHead().insertRow();
        for (const label of ['Base', '1 · ä', '2 · u', '3 · i', '4 · a', '5 · ē', '6 · ə', '7 · o', 'Other']) {
            const cell = document.createElement('th');
            cell.scope = 'col'; cell.textContent = label; header.append(cell);
        }
        const body = table.createTBody();
        const { rows, extras } = ethiopicRows(keys);
        for (const keys of rows) {
            const row = body.insertRow();
            const label = document.createElement('th');
            label.scope = 'row'; label.textContent = keys[0][0];
            label.title = keys[0][1]; row.append(label);
            for (const key of keys) {
                const cell = row.insertCell();
                if (key) cell.append(makeKey(key));
                else { cell.textContent = '—'; cell.setAttribute('aria-label', 'No character'); }
            }
        }
        const extraRow = body.insertRow();
        const extraCell = extraRow.insertCell(); extraCell.colSpan = 9;
        const label = document.createElement('p'); label.textContent = 'Additional forms, punctuation and numbers';
        extraCell.append(label, ...extras.map(makeKey));
        el('keyboardKeys').replaceChildren(table);
    } else el('keyboardKeys').replaceChildren(...keys.map(makeKey));
    const marks = layout[3].find(([name]) => /^(Vowel points|Accents and breathings|Combining marks)$/.test(name));
    el('keyboardMarksArea').hidden = !!query || !marks || marks === groups[0];
    el('keyboardMarks').replaceChildren(...(marks?.[1] || []).map(makeKey));
    el('keyboardCount').textContent = keys.length ? `${keys.length} characters` : 'No matching characters';
}
function changeLanguage() {
    const layout = layouts.find(layout => layout[0] === el('keyboardLanguage').value);
    el('keyboardGroup').closest('label').hidden = layout[0] === 'ethiopic';
    el('keyboardGroup').replaceChildren(...layout[3].map(([name], i) => new Option(name, i)));
    el('keyboardSearch').value = '';
    renderKeys();
}
el('keyboardLanguage').replaceChildren(...layouts.map(([id, name]) => new Option(name, id)));
el('keyboardLanguage').onchange = changeLanguage;
el('keyboardGroup').onchange = renderKeys;
el('keyboardSearch').oninput = renderKeys;
// Palette controls are not transcription edits and must not trigger unsaved-review prompts.
el('unicodeKeyboard').addEventListener('input', event => event.stopPropagation());
el('keyboardCodepoint').addEventListener('keydown', event => {
    if (event.key === 'Enter') { event.preventDefault(); el('insertCodepoint').click(); }
});
el('keyboardSearch').addEventListener('keydown', event => { if (event.key === 'Enter') event.preventDefault(); });
bindButton(el('insertCodepoint'), () => { try { insert(fromCodePoint(el('keyboardCodepoint').value)); } catch (error) { status(error.message, true); } });
bindButton(el('keyboardSpace'), () => insert(' '));
bindButton(el('keyboardBackspace'), () => apply(backspace));
bindButton(el('keyboardUndo'), () => {
    if (el('save').disabled) return;
    const previous = history.pop();
    if (!previous || previous.field.value !== previous.after.value) {
        history = []; status('No unchanged keyboard edit to undo.'); return;
    }
    target = previous.field;
    target.value = previous.before.value;
    target.focus({ preventScroll: true });
    target.setSelectionRange(previous.before.start, previous.before.end);
    target.dispatchEvent(new Event('input', { bubbles: true }));
    status('Keyboard edit undone.');
});
// New line loads and completed saves establish a new editing context.
for (const event of ['transcription-rendered', 'transcription-runs-rendered']) document.addEventListener(event, () => { history = []; target = defaultTarget(); updateTarget(); status(''); });
changeLanguage();
updateTarget();
})();
