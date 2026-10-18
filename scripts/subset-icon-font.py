"""入力モードのアイコン用に、M PLUS Rounded 1c Regular を「あ」「A」「×」だけにサブセット化する。

使い方:
    python scripts/subset-icon-font.py <MPLUSRounded1c-Regular.ttf> [出力先]

元のフォントは Google Fonts の google/fonts リポジトリの
ofl/mplusrounded1c/MPLUSRounded1c-Regular.ttf。出力先の既定は
tsf/src/Font/MPLUSRounded1c-Regular-icon.ttf (DLL のリソースに入る)。
"""

import sys
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont

TEXT = "あA×"


def main() -> int:
    if len(sys.argv) not in (2, 3):
        print(__doc__, file=sys.stderr)
        return 2
    source = Path(sys.argv[1])
    if len(sys.argv) == 3:
        output = Path(sys.argv[2])
    else:
        output = Path(__file__).resolve().parent.parent / "tsf" / "src" / "Font" / "MPLUSRounded1c-Regular-icon.ttf"

    options = subset.Options()
    # 著作権・ライセンスの表記 (name ID 0・13・14) を落とさないよう、name table は全部残す
    options.name_IDs = ["*"]
    options.name_languages = ["*"]
    options.layout_features = []
    font = TTFont(source)
    subsetter = subset.Subsetter(options)
    subsetter.populate(text=TEXT)
    subsetter.subset(font)
    output.parent.mkdir(parents=True, exist_ok=True)
    font.save(output)

    missing = [ch for ch in TEXT if ord(ch) not in font.getBestCmap()]
    if missing:
        print(f"フォントに無い字: {missing}", file=sys.stderr)
        return 1
    print(f"{output}: {output.stat().st_size} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
