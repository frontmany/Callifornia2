from __future__ import annotations

from pathlib import Path
import xml.etree.ElementTree as ET

import cv2
import numpy as np


SOURCE_DIR = Path(r"C:\Users\admin\Desktop\p")
OUTPUT_DIR = Path(r"C:\prj\Callifornia2\client\icons\svg")
FILL_COLOR = "#FFFFFF"
ALPHA_THRESHOLD = 10
MIN_CONTOUR_AREA = 4.0
EPSILON_FACTOR = 0.002


def contour_to_path(contour: np.ndarray) -> str:
    points = contour.reshape(-1, 2)
    if len(points) == 0:
        return ""
    start = points[0]
    commands = [f"M {int(start[0])} {int(start[1])}"]
    for p in points[1:]:
        commands.append(f"L {int(p[0])} {int(p[1])}")
    commands.append("Z")
    return " ".join(commands)


def extract_mask(image: np.ndarray) -> np.ndarray:
    if image.ndim == 3 and image.shape[2] == 4:
        alpha = image[:, :, 3]
        return (alpha > ALPHA_THRESHOLD).astype(np.uint8) * 255

    if image.ndim == 3:
        gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)
    else:
        gray = image
    _, mask = cv2.threshold(gray, 245, 255, cv2.THRESH_BINARY_INV)
    return mask


def convert_png_to_svg(png_path: Path, svg_path: Path) -> bool:
    image = cv2.imread(str(png_path), cv2.IMREAD_UNCHANGED)
    if image is None:
        return False

    if image.ndim == 2:
        h, w = image.shape
    else:
        h, w = image.shape[:2]

    mask = extract_mask(image)

    contours, hierarchy = cv2.findContours(
        mask, cv2.RETR_CCOMP, cv2.CHAIN_APPROX_SIMPLE
    )
    if not contours:
        return False

    path_parts: list[str] = []
    for contour in contours:
        area = cv2.contourArea(contour)
        if area < MIN_CONTOUR_AREA:
            continue
        epsilon = EPSILON_FACTOR * cv2.arcLength(contour, True)
        simplified = cv2.approxPolyDP(contour, epsilon, True)
        path_data = contour_to_path(simplified)
        if path_data:
            path_parts.append(path_data)

    if not path_parts:
        return False

    svg = ET.Element(
        "svg",
        {
            "xmlns": "http://www.w3.org/2000/svg",
            "width": str(w),
            "height": str(h),
            "viewBox": f"0 0 {w} {h}",
        },
    )
    ET.SubElement(
        svg,
        "path",
        {
            "fill": FILL_COLOR,
            "fill-rule": "evenodd",
            "d": " ".join(path_parts),
        },
    )

    tree = ET.ElementTree(svg)
    tree.write(svg_path, encoding="utf-8", xml_declaration=True)
    return True


def main() -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    png_files = sorted(SOURCE_DIR.glob("*.png"))
    if not png_files:
        print(f"No PNG files found in: {SOURCE_DIR}")
        return

    converted = 0
    for png in png_files:
        target = OUTPUT_DIR / f"{png.stem}.svg"
        if convert_png_to_svg(png, target):
            converted += 1
            print(f"OK  {png.name} -> {target.name}")
        else:
            print(f"SKIP {png.name}")

    print(f"Converted {converted}/{len(png_files)} files into {OUTPUT_DIR}")


if __name__ == "__main__":
    main()
