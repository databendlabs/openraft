#!/usr/bin/env python3

from pathlib import Path

def get_section_name(dir_path):
    readme = dir_path / "README.md"
    return readme.read_text().strip()

def build_faq():
    output = []

    category_dirs = sorted([d for d in Path(".").iterdir() if d.is_dir() and d.name[0].isdigit()])

    for dir_path in category_dirs:
        section_name = get_section_name(dir_path)
        output.append(f"## {section_name}\n\n")

        md_files = sorted([f for f in dir_path.glob("*.md") if f.name != "README.md"])

        for md_file in md_files:
            content = md_file.read_text()
            output.append(content)
            output.append("\n\n")

    Path("faq.md").write_text("".join(output))
    print("Generated faq.md successfully")

if __name__ == "__main__":
    build_faq()
