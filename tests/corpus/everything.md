---
title: Everything
tags: [markdown, test]
---

# Heading one

Some *emphasis*, **strong**, ~~struck~~, `inline code`, and a [link](https://example.com) plus <https://autolink.example>. A footnote[^1] too. This sentence is long enough that it has to wrap at least once at sixty columns.

## Heading two

### Heading three with `code`

Line one  
hard break, then<br>an html break.

- Bullet one
- Bullet two with a longer body that wraps onto another line when narrow
  - Nested
    - Deeper
- [ ] Todo
- [x] Done

1. First
2. Second

   Loose paragraph in item two.

10. Ten
11. Eleven

> A quote
>
> > nested quote

> [!WARNING]
> Careful now.

```rust
fn main() {
	println!("hello, a fairly long line that will need wrapping somewhere");
}
```

    indented code

| Left | Center | Right |
|:-----|:------:|------:|
| a | b | c |
| a longer cell that wraps | 日本語 | 42 |

---

Term
: Its definition.

<!-- hidden comment -->

<div align="center">shown html</div>

<p align="center">
  <img alt="Logo" src="logo.svg" width="300">
</p>
<p align="center"><a href="x"><img src="b.svg" alt="build"></a> <a href="y"><img alt="license"></a></p>

Inline <kbd>Ctrl</kbd> and a<br/>break.

![alt text](img/logo.png)

[^1]: The footnote body.
