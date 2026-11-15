# `.lev` files — encoded configuration, level, pack, and high-score scripts

Status: **verified** for Gold 1.20, Gold 1.15, and 1.51d. Every `.lev` file in all three decodes, and the decoded length equals the declared length. The decoder is `tools/lev_decode.py`.

## Lookup

The game refers to files by their XML name, for example `config.xml`, `bug1.xml`, `pack*.xml`, or `highscores.xml`. It opens `<name>` + `.lev` (suffix string at `0x43822c` in Gold 1.20). Windows file lookup is case-insensitive, and shipped names mix case (`BUG1.XML.lev`, `bug14.xml.lev`). **A port must do a case-insensitive lookup.**

## Encoding (Gold 1.20 addresses)

1. Read the whole file.
2. If the global at `0x4370a4` is nonzero (its initial value is 1), apply `byte = ~byte` to every byte. This happens in the loop in `FUN_00417fe0` and its siblings.
3. Decompress with `FUN_004076a0`:
   - `FUN_0041ce10` checks the `u16` magic `0x434A` ("JC") and returns the `u32` decompressed size at offset 2.
   - It allocates size + 4 bytes (`FUN_0041e916`).
   - `FUN_0041cfae` decompresses the bitstream starting at offset 6.
4. The bitstream is JCALG1-style LZ77 (hypothesis based on the magic and structure; the port is verified by output):
   - Tag bits are read MSB-first from little-endian 32-bit words.
   - Token types are literal (with adaptive literal width and bias), gamma-coded match, repeat-last-offset match, short 7-bit-offset match, one-byte copy and zero byte, and 256-byte raw blocks.
   - Match lengths get a bonus from offset thresholds: `0x7F`, `0x27E`, `0x37FF`, `0x10000`.
   - See `tools/lev_decode.py` for the exact port.

## Decoded content

The decoded data is **XML-like text with CRLF line endings, not valid XML**:

- Tags are usually unclosed (`<MODEL ...>` without `/>`). Some closing tags exist (`</ALIEN>`, `</MODELS>`, `</FONTS>`).
- Attributes can repeat. For example, `FILE_NAME="pinch1.r3d" FILE_NAME="n4b.r3d"` appears on one tag. Which one wins is **unresolved** and depends on the scanner.
- `<!STAGE …>` and `<!POINT …>` comment out a line.
- Numbers can omit the leading zero (`.995`). Colours can be hex (`COLOR3="BBBB88"`).
- The engine's parser (`FUN_00419bb0`, `FUN_00403730`, `FUN_0040ca60`, and others) is a custom scanner. **The port must reimplement that scanner's semantics, not use an XML library.** Tests should cover duplicate attributes, comments, and field-extraction edge cases such as `FUN_00417f90`, which copies until `\r`.

### Top-level structure of a level file (`bugN.xml`)

```
<WAVE STARS=".." SCORE="..">
  <MUSIC NAME="ky_ck.xm"> </MUSIC>
  <MODELS> <MODEL NAME=.. FILE_NAME=..r3d [SCALE] [TIMES] [RED1..BLUE3|COLOR1..3]> ... </MODELS>
  <FONTS>  <FONT FACE=WH|NJ|SCORE|POPUP|LIVES NAME=.. SCALE SPACING [COLOR|RED1..BLUE2] [ALPHA_SCALE]> ... </FONTS>
  <SHIP MODEL BULLET_MODEL FIRE_SOUND SHIP_SPAWN_SOUND EXPLO_SOUND CAPTURE_SOUND ...> </SHIP>
  <POWER_UPS ...> <ACCELLERATION|ADD_BULLET|ATOMS|BOING|CAPTURE|DECELLERATION|DIRECTIONAL|EXTRA_LIFE|
                   FIREBALL|GROW_ALIENS|GUIDED|SHRINK_ALIENS|SPINNER|SUBTRACT_BULLET|WRECKING_BALL ...>
  <LEVEL SCORE_FONT CLUSTER_FONT POWER_UP_FONT LIVES_FONT>      (repeated, 4–9 per file)
     <TEXT FONT LETTERS IN_EFFECT OUT_EFFECT MODIFIER IN_TIME OUT_TIME START_TIME IDLE_TIME
           IN_MOVEMENT OUT_MOVEMENT IDLE_MOVEMENT Y ...>
     <ALIEN DORMANT_TICKS MODEL POINTS X Y Z SETTLE_TIME SETTLE_TIME_VARIANCE DECAY EXPLO_SOUND
            BULLET_MODEL FIRE_FREQUENCY FIRE_SOUND EXPLOSION_COLOR ...>
        <STAGE BANK TYPE=ALIEN_* NUMBER [CLUSTER]>     (per-bank state sequence)
        <POINT TYPE=ATTACK|SETTLE|DIVE SPEED ACCELLERATION X Y Z RADIUS [XDIR YDIR ZDIR ROTATE_TYPE]>
     </ALIEN>
  </LEVEL>
  <NEXT_CONFIG NAME="bug2.xml">
```

The full attribute and enum vocabulary is in the executable's string table: about 40 `ALIEN_*` stage types, `ROTATE_*`, `COLOR_*`, `OFFSET_*`, and text effects. It still needs to be catalogued with each consuming function.

### Chaining (verified from data)

Each pack has four files linked by `NEXT_CONFIG`, ending at `highscores.xml`:

| Pack | Files |
|---|---|
| 1 | bug1 → bug2 → bug3 → bug4 |
| 2 | bug5 → … → bug8 |
| 3 | bug9 → … → bug12 |
| 4 | bug14 → … → bug17 |
| 5 | bug18 → … → bug21 |
| 6 | bug13, a single file |

`config.xml` is the attract and menu config. It contains `LINK_*` and `PROCESS_*` menu actions.

### Pack manifests (`packN.xml`)

These are plain lines rather than tags:

1. Display name, for example `SHAREWARE LEVELS` or `GOLD PACK BONUS LEVELS`.
2. A number: 16, or 8 for pack 6. Hypothesis: the displayed level count. In every pack it equals the total `<LEVEL>` count minus 1.
3. Difficulty label: `MEDIUM`, `HARD`, or `OUCH`.
4. First level file.
5. `eof`.
6. Padding lines of `X`.

The pack-scan code, `FUN_00417fe0` with `FindFirstFileA("pack*.xml")`, extracts fields with `FUN_00417f90`.
