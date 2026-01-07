# mdview Design Principles

> "Less, but better." — Dieter Rams

This document establishes the design principles that guide mdview toward becoming the world's finest terminal markdown viewer. These principles are grounded in typography research, cognitive science, and a century of design wisdom.

---

## The Core Philosophy

**mdview exists to disappear.**

The best reading experience is one where the reader forgets they're using software and becomes immersed in the content. Every design decision must serve this singular purpose: removing friction between the reader and the text.

We are not building a code editor, a syntax highlighter, or a markdown debugger. We are building a **reading environment** — a quiet room where words can breathe and ideas can flow uninterrupted into the reader's mind.

---

## The Ten Principles

### 1. Respect the Measure

> *"Anything from 45 to 75 characters is widely regarded as a satisfactory length of line for a single-column page set in a serifed text face in a text size. The 66-character line (counting both letters and spaces) is widely regarded as ideal."*
> — Robert Bringhurst, The Elements of Typographic Style

**The Principle:** Line length is not aesthetic preference — it is cognitive ergonomics. The human eye can only track a limited horizontal distance before losing its place. Too wide, and readers struggle to find the next line. Too narrow, and reading becomes fragmented.

**In Practice:**
- Body text: 66 characters (the Bringhurst number)
- Acceptable range: 45-75 characters
- Never exceed 80 characters (WCAG accessibility guideline)
- Monospace terminals make this calculable: 66 columns is 66 characters

**The Science:**
- Baymard Institute found content wider than 80 characters was skipped 41% more often
- Eye-tracking research shows longer lines increase saccades (eye movements) and reader fatigue
- The British Dyslexia Association recommends 60-70 characters for improved accessibility

---

### 2. Let Whitespace Breathe

> *"Whitespace is to be regarded as an active element, not a passive background."*
> — Jan Tschichold

**The Principle:** Whitespace is not emptiness — it is structure. It groups related content through proximity, separates distinct ideas, and gives the eye places to rest. The space between elements communicates as much as the elements themselves.

**In Practice:**
- Generous margins around the content column
- Consistent spacing between paragraphs (1 blank line)
- Increased spacing before headers (2-3 blank lines before major headers)
- Top and bottom padding to frame the content like a book page
- Never fill space just because it's available

**The Science:**
- Gestalt principle of proximity: items close together are perceived as related
- Studies show cluttered interfaces increase cognitive load by up to 50%
- Magazine research (Kinfolk, etc.) demonstrates dramatic whitespace increases perceived quality

---

### 3. Establish Vertical Rhythm

> *"The baseline grid is the foundation of vertical rhythm."*

**The Principle:** Consistent vertical spacing creates a sense of reliability, integrity, and design quality. Text should flow in predictable intervals, like music with a steady beat. This rhythm reduces the cognitive effort of reading.

**In Practice:**
- Define a base unit (e.g., 1 line = 1 unit)
- All vertical spacing should be multiples of this unit
- Paragraph spacing: 1 unit
- Section spacing: 2-3 units
- Header spacing above: 2-3 units for major (H1/H2), 2 units for minor (H3+)
- Never use arbitrary spacing values

**The Science:**
- Consistent spacing helps readers predict where the next element will appear
- The 8-point grid system is widely adopted in modern UI design for this reason
- Vertical rhythm reduces eye strain during extended reading sessions

---

### 4. Create Clear Hierarchy

> *"Visual hierarchy is the order in which humans process information on a page."*

**The Principle:** Not all content is equal. Headers should immediately signal their importance level. The reader should instantly understand the structure of the document through visual cues alone — without reading the words.

**In Practice:**
- H1: Maximum prominence (uppercase, bold, warmest color)
- H2: Strong prominence (bold, warm color)
- H3: Moderate prominence (bold, slightly muted)
- H4-H6: Progressively subtler distinction
- Body text: Neutral, comfortable, unobtrusive
- Maximum 3 levels of contrast (research shows more creates confusion)

**The Science:**
- Effective hierarchy reduces time-to-comprehension
- Users can scan structured documents 47% faster (NNGroup research)
- Typography hierarchy has three pillars: size/weight, color, and spacing

---

### 5. Choose Warmth Over Harshness

> *"Warmer lights, which have more light at the red end of the spectrum, are easier on the eye than cooler blue/white lights."*

**The Principle:** Sustained reading demands colors that don't fatigue the eye. Pure black on pure white creates harsh contrast. Blue light strains the eyes. Warm, muted tones invite the reader to stay longer.

**In Practice:**
- Avoid pure black (#000000) and pure white (#FFFFFF)
- Body text: Respect the user's terminal colors (they've chosen what works for them)
- Accents: Warm browns, ambers, and muted earth tones
- Borders and rules: Very subtle, almost invisible
- Target contrast ratio: 7:1+ for body text (WCAG AAA), but avoid exceeding ~15:1

**The Science:**
- PMC research found yellow/warm text causes lowest visual fatigue; red text causes highest
- Color temperatures of 2500-4000K are most comfortable for extended reading
- High contrast (7:1+) ensures accessibility without causing eye strain

---

### 6. Minimize Cognitive Load

> *"The average person can only keep 7 (plus or minus 2) items in their working memory."*
> — George Miller

**The Principle:** Every visual element demands mental processing. Every color, every border, every decoration consumes cognitive resources that could be spent understanding the content. Remove everything that doesn't directly serve comprehension.

**In Practice:**
- No unnecessary ornamentation
- No competing elements
- No animations or motion
- Interface elements (status bar) should be DIM by default
- Progressive disclosure: show information only when needed
- Use familiar patterns (users shouldn't have to learn anything)

**The Science:**
- John Sweller's Cognitive Load Theory (1980s) established that learning degrades when extraneous load increases
- Research shows reducing cognitive load by 77% increased task completion by 25%
- Medium's minimalist design is cited as a gold standard for focused reading

---

### 7. Honor the Monospace Constraint

> *"Monospace fonts, characterized by uniform spacing where each character occupies the same fixed width, offer a visually consistent and structured layout."*

**The Principle:** Terminal typography operates under unique constraints. Every character is the same width. We cannot kern, we cannot use serifs, we cannot adjust letter spacing. These constraints become our strength — they enforce alignment and create a distinctive aesthetic.

**In Practice:**
- Embrace the grid — everything aligns naturally
- Use box-drawing characters (│ ─ ┌ ┐ └ ┘) for elegant borders
- Column width calculations are exact (66 characters = 66 columns)
- Indentation is predictable and precise
- Let the monospace aesthetic be part of the identity, not something to fight against

**The Benefit:**
- Perfect alignment without effort
- Predictable wrapping behavior
- Tables and code naturally align
- Character-counting tools work perfectly

---

### 8. Design for Disappearance

> *"Good design is as little design as possible."*
> — Dieter Rams

**The Principle:** The interface should be invisible. When someone finishes reading in mdview, they should remember the content — not the tool. The highest praise is when users don't consciously notice the design at all.

**In Practice:**
- No splash screens or branding during use
- Navigation hints appear only when needed
- Status bar is minimal and dim
- End-of-document marker is subtle (· · ·)
- No features that call attention to themselves
- The design should feel "obvious" and "natural"

**Instapaper's Philosophy:**
> "The most comfortable reading experience... enforces a mental focus that is very relaxing."

Reading apps that succeed (Kindle, Instapaper, Readwise) share one trait: they create a "quiet room" where distractions cease to exist.

---

### 9. Maintain Ruthless Consistency

> *"Good design is consistent in every detail."*
> — Dieter Rams

**The Principle:** Inconsistency creates cognitive friction. If bullets look one way in one place and different elsewhere, the reader's brain must process this discrepancy. Every inconsistency, no matter how small, erodes trust and increases fatigue.

**In Practice:**
- One style for bullets (• everywhere)
- Consistent spacing rules (never "approximately")
- Same color for same semantic meaning throughout
- Borders always use the same characters
- If something appears twice, it must appear identically

**The Details That Matter:**
- List markers: always "•" for unordered, always "1." style for ordered
- Blockquote borders: always "│" on the left
- Code block borders: always "╭ ╰ │ ─"
- Horizontal rules: always "─ · ─" centered

---

### 10. Serve the Reader, Not the Author

**The Principle:** Markdown is a writing format. mdview is a reading tool. The distinction matters. Writers need syntax visible. Readers need syntax invisible. We optimize entirely for the reading experience, even when it means departing from what markdown "looks like" in an editor.

**In Practice:**
- Hide markdown syntax completely (no `#` before headers)
- Transform inline code markers to subtle visual styling (‹code›)
- Render links as "text [→ url]" rather than raw markdown
- Tables become beautiful, not raw pipes and dashes
- The reader should never see markdown — only its meaning

**The Philosophy:**
A great editorial designer doesn't show readers the printing press. They show readers the story. mdview transforms markdown into editorial typography, just as a printing press transforms manuscript into book.

---

## Technical Specifications

### Typography Constants

| Element | Value | Rationale |
|---------|-------|-----------|
| Optimal line width | 66 characters | Bringhurst's golden measure |
| Maximum line width | 76 characters | Emergency fallback for narrow terminals |
| Minimum margin | 2 characters | Ensure text never touches edge |
| Paragraph spacing | 1 blank line | Standard editorial convention |
| Major header spacing | 3 blank lines above | Clear section breaks |
| Minor header spacing | 2 blank lines above | Subsection differentiation |

### Contrast Guidelines

| Element | Contrast Ratio | WCAG Level |
|---------|---------------|------------|
| Body text | 7:1+ | AAA |
| Secondary text | 4.5:1+ | AA |
| Decorative elements | 3:1+ | AA (UI components) |
| Disabled/dim text | No minimum | (intentionally subtle) |

### Color Temperature

| Context | Color Temperature | Notes |
|---------|------------------|-------|
| Body text | User's terminal default | Respect user preference |
| Headers | Warm browns (2500-3500K equivalent) | Inviting, not harsh |
| Borders | Cool neutrals | Recede visually |
| Accents | Warm earth tones | Gruvbox-inspired palette |

---

## Anti-Patterns to Avoid

1. **Feature creep** — Adding capabilities that serve edge cases at the cost of simplicity
2. **Syntax highlighting body text** — Code gets highlighting; prose does not
3. **Competing colors** — More than 3-4 distinct colors creates visual chaos
4. **Dense layouts** — Fear of whitespace leads to cluttered, exhausting interfaces
5. **Inconsistent spacing** — "About 2 lines" is never acceptable; be precise
6. **Fighting the terminal** — Work with monospace constraints, not against them
7. **Ornamentation** — Decorative elements that serve no comprehension purpose
8. **Over-communication** — Status bars that constantly update and distract
9. **Dark patterns** — Any element that draws attention to the tool rather than content
10. **Premature optimization** — Adding features before core reading experience is perfect

---

## The Test

Before any design decision, ask:

1. **Does this help the reader understand the content?**
   - If no → remove it

2. **Could this be simpler?**
   - If yes → simplify it

3. **Is this consistent with everything else?**
   - If no → make it consistent

4. **Would a reader notice this?**
   - If yes → it's probably too prominent

The ultimate test: Can someone read a 10,000-word essay in mdview and afterward only remember what the essay said — not what the tool looked like?

---

## Inspirations and References

### Books
- *The Elements of Typographic Style* — Robert Bringhurst
- *Thinking With Type* — Ellen Lupton
- *The Design of Everyday Things* — Don Norman
- *Dieter Rams: Ten Principles for Good Design*

### Design Systems
- Gruvbox color palette
- Tufte CSS for web typography
- Kindle's reading interface
- Instapaper's focus on distraction-free reading

### Research Sources
- [Baymard Institute — Line Length Readability](https://baymard.com/blog/line-length-readability)
- [WCAG 2.1 Contrast Guidelines](https://www.w3.org/WAI/WCAG21/Understanding/contrast-minimum.html)
- [NNGroup — Minimize Cognitive Load](https://www.nngroup.com/articles/minimize-cognitive-load/)
- [Interaction Design Foundation — Visual Hierarchy](https://www.interaction-design.org/literature/topics/visual-hierarchy)
- [Smashing Magazine — Designing for the Reading Experience](https://www.smashingmagazine.com/2013/02/designing-reading-experience/)
- [PMC — Effect of Text Color on Visual Fatigue](https://pmc.ncbi.nlm.nih.gov/articles/PMC11175232/)

---

## Closing Thought

> *"In the new computer age, the proliferation of typefaces and type manipulations represents a new level of visual pollution threatening our culture. Out of thousands of typefaces, all we need are a few basic ones, and trash the rest."*
> — Massimo Vignelli

In a world of visual noise, mdview offers silence. In a sea of features, we offer focus. In an age of distraction, we offer a quiet room for reading.

The world's best markdown viewer isn't the one with the most features. It's the one that lets you forget you're using a markdown viewer at all.

---

*This document should be treated as a living guide. When design decisions arise, return here. When features are proposed, test them against these principles. When in doubt, choose simplicity.*
