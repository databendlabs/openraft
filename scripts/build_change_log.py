#!/usr/bin/env python

import os
import re
import subprocess
import sys

import semantic_version
import toml
import yaml

with open("scripts/change-types.yaml", "r") as f:
    typs = yaml.safe_load(f.read())

typs = {x: x for x in typs}

# categories has another mapping to fix typo in commit message
categories = {
    "data-change:": typs["data-change"],
    "data-changes:": typs["data-change"],
    "DataChange:": typs["data-change"],
    "DataChanges:": typs["data-change"],
    "api-change:": typs["api-change"],
    "new-feature:": typs["new-feature"],
    "improve:": typs["improve"],
    "Improve:": typs["improve"],
    "Perf:": typs["improve"],
    "perf:": typs["improve"],
    "internal:": typs["internal"],
    "doc:": typs["doc"],
    "Doc:": typs["doc"],
    "refactor:": typs["refactor"],
    "fixbug:": typs["fixbug"],
    "fixdoc:": typs["fixdoc"],
    "Fixdoc:": typs["fixdoc"],
    "dep:": typs["dep"],
    "ci:": typs["ci"],
    "CI:": typs["ci"],
    # fix typo
    "Change:": typs["api-change"],
    "change:": typs["api-change"],
    "changes:": typs["api-change"],
    "api-changes:": typs["api-change"],
    "Add:": typs["new-feature"],
    "add:": typs["new-feature"],
    "Feature:": typs["new-feature"],
    "feature:": typs["new-feature"],
    "Features:": typs["new-feature"],
    "features:": typs["new-feature"],
    "feat:": typs["new-feature"],
    "new-features:": typs["new-feature"],
    "docs:": typs["doc"],
    "fix:": typs["fixbug"],
    "Fix:": typs["fixbug"],
    "fixup:": typs["fixbug"],
    "test:": typs["test"],
    "Dep:": typs["dep"],
    "deps:": typs["dep"],
    "build(deps):": typs["dep"],
    "Build(deps):": typs["dep"],
    "Update:": typs["other"],
    "update:": typs["other"],
    "turn:": typs["other"],
    "replace:": typs["refactor"],
    "format:": typs["refactor"],
    "use:": typs["refactor"],
    "Create:": typs["other"],
    "BumpVer:": typs["other"],
    "Chore:": typs["other"],
    "chore:": typs["other"],
}

category_display = {
    "data-change": "DataChanged",
    "api-change": "Changed",
    "new-feature": "Added",
    "improve": "Improved",
    "dep": "Dependency",
    "fixbug": "Fixed",
    "fixdoc": "DocFixed",
}

replace_subjects = [
    (r"^([^: ]+) ", r"\1: "),  # add ":" if not
    (r"^(\w+:) *", r"\1 "),  # 0 or many space to 1 space
    (r"^build\(dpes\): ", r"dep: "),
]

ignores = [
    "^Merge pull request",
    "^BumpVer:",
]

to_display = {
    "other": False,
    "doc": False,
    "refactor": False,
    "internal": False,
    "test": False,
    "ci": False,
}

commit_url_ptn = "https://github.com/databendlabs/openraft/commit/{hash}"


def cmd(cmds):
    subproc = subprocess.Popen(
        cmds,
        encoding="utf-8",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    out, err = subproc.communicate()
    subproc.wait()

    code = subproc.returncode
    if code != 0:
        raise OSError(out + "\n" + err)

    return out


def list_tags():
    out = cmd(["git", "tag", "-l"])
    tags = out.splitlines()
    tags[0].lstrip("v")
    tags = [
        semantic_version.Version(t.lstrip("v")) for t in tags if t != "" and t != "base"
    ]
    return tags


def changes(frm, to):
    # subject, author time, author name, email.

    git_cmd = ["git", "merge-base", frm, to]
    common_base = cmd(git_cmd).strip()
    print(f"--- Find common base {common_base} of {frm} {to} with: {git_cmd}")

    git_cmd = ["git", "log", "--format=%H", "--reverse", common_base + ".." + to]
    print("--- List commits with: {git_cmd}".format(git_cmd=" ".join(git_cmd)))

    commits = cmd(git_cmd)
    commits = commits.splitlines()

    rst = []

    for commit in commits:
        # Extract subject, date, author and email
        out = cmd(["git", "log", "-1", "--format=%s ||| %ai ||| %an ||| %ae", commit])
        line = out.strip()

        if if_ignore(line):
            continue

        line = replace_subject(line)

        elts = line.split(" ||| ")

        body = cmd(["git", "log", "-1", "--format=%b", commit])

        item = {
            "hash": commit,
            "subject": elts[0],
            # 2019-04-18 13:36:42 +0800
            "time": elts[1].split()[0],
            "author": elts[2],
            "email": elts[3],
            "body": body,
        }

        rst.append(item)

    return rst


def replace_subject(line):
    output = line
    for ptn, repl in replace_subjects:
        output = re.sub(ptn, repl, output)

    if output != line:
        print("--- Fix commit message, replace", ptn, "with", repl)
        print("input: ", line)
        print("output:", output)
        print()
    return output


def if_ignore(line):
    for ign in ignores:
        if re.match(ign, line):
            print("--- Ignore trivial commit by pattern:", ign)
            print("Ignored:", line)
            print()
            return True
    return False


def norm_changes(changes):
    """
    Collect changes by category.
    Indent commit message body.
    """
    rst = {}
    for ch in changes:
        print("--- Normalize subject:", ch["hash"], ch["subject"])
        sub = ch["subject"]
        cate, cont = (sub.split(" ", 1) + [""])[:2]
        catetitle = categories.get(cate, typs["other"])

        if catetitle not in rst:
            rst[catetitle] = []

        c = rst[catetitle]
        bodylines = ch["body"].strip().splitlines()
        bodylines = ["    " + x for x in bodylines]
        desc = {
            "hash": ch["hash"],
            "content": cont,
            "time": ch["time"],
            "author": ch["author"],
            "email": ch["email"],
            "body": bodylines,
        }
        c.append(desc)

    return rst


def build_ver_changelog(new_ver, commit="HEAD", since=None):
    """
    Build change log for ``new_ver`` at ``commit``.
    It will find out all commit since the last tag that is less than ``new_ver``

    If ``since`` is specified, build change log since ``since`` up to ``commit``.
    """

    fn = version_chaagelog_fn(new_ver)
    if os.path.exists(fn):
        print(f"--- Version {new_ver} change log exists, skip...")
        print(f"--- To rebuild it, delete {fn} and re-run")
        return

    if since is None:
        tags = list_tags()
        tags.sort()

        new_ver = new_ver.lstrip("v")
        new_ver = semantic_version.Version(new_ver)
        tags = [t for t in tags if t < new_ver]
        latest = "v" + str(tags[-1])
    else:
        latest = since

    chs = changes(latest, commit)
    chs = norm_changes(chs)

    lines = []
    for cate, descs in chs.items():
        if not to_display.get(cate, True):
            continue

        cate = category_display[cate]

        cate_title = f"### {cate}:"
        lines.append(cate_title)
        lines.append("")

        for desc in descs:
            short_hash = desc["hash"][:8]
            url = commit_url_ptn.format(**desc)

            title = "-   {cate}: [{short_hash}]({url}) {content}; by {author}; {time}".format(
                cate=cate, short_hash=short_hash, url=url, **desc
            )
            lines.append(title)
            lines.append("")
            if len(desc["body"]) > 0:
                lines.extend(desc["body"])
                lines.append("")

    # remove blank
    lines = [x.rstrip() for x in lines]

    changelog = "\n".join(lines)

    with open(fn, "w") as f:
        f.write(changelog)


def build_ver_changelog_summary(ver):
    """
    Build summary in short list
    Summary:

    - Added:
        -   Define custom `Entry` type for raft log.
        -   Add feature flag storage-v2 to enable `RaftLogStorage` and `RaftStateMachine`.

    Detail:
    """

    fn = version_chaagelog_fn(ver)

    header = "Summary:"
    footer = "Detail:"

    with open(fn, "r") as f:
        lines = f.readlines()

    lines = [line.rstrip() for line in lines]

    # Remove existent summary
    try:
        footer_indexes = lines.index(footer)
    except ValueError:
        print("No footer found")
    else:
        # skip `Detail:`, and a following blank line.
        lines = lines[footer_indexes + 2 :]

    # Build summary:
    # - Remove lines that starts with space, which is detail section, a list item
    #   with indent.
    # - Remove blank lines
    source = [line for line in lines if not line.startswith(" ") and line != ""]
    summary = [header, ""]
    for line in source:
        if line.startswith("### "):
            # Section header, Strip "### "
            line = "- " + line[4:]

        elif line.startswith("-   "):
            # Log:
            #   remove `Fixed:`,
            line = line.split(sep=None, maxsplit=2)[2]

            # First letter upper case
            if line[0] >= "a" and line[0] <= "z":
                line = line[0].upper() + line[1:]

            # Remove suffix date
            line = re.sub(r"; \d{4}-\d{2}-\d{2}$", "", line)

            # Remove suffix author
            line = re.sub("; by .*?$", ".", line)

            # Remove redundant "."
            line = re.sub("[.][.]$", ".", line)

            #   add indent:
            line = "    -   " + line
        else:
            raise ValueError("unknown line: " + repr(line))

        summary.append(line)

    summary.append("")
    summary.append(footer)
    summary.append("")

    with open(fn, "w") as f:
        for line in summary:
            f.write(line + "\n")
        for line in lines:
            f.write(line + "\n")


def version_chaagelog_fn(ver):
    _ = semantic_version.Version(ver)

    fn = f"change-log/v{ver}.md"

    return fn


def build_changelog():
    out = cmd(["ls", "change-log"])
    vers = out.splitlines()
    # remove suffix "md"
    vers = [x.rsplit(".", 1)[0] for x in vers if x != ""]
    vers.sort(key=lambda x: semantic_version.Version(x.lstrip("v")))

    with open("change-log.md", "w") as f:
        for v in reversed(vers):
            print(f"--- append change log of {v}")

            f.write("## " + v + "\n\n")
            with open(f"change-log/{v}.md", "r") as vf:
                cont = vf.read()

            cont = cont.splitlines()
            cont = "\n".join(cont)

            f.write(cont + "\n\n")


def load_cargo_version():
    with open("./openraft/Cargo.toml", "r") as f:
        cargo = f.read()

    t = toml.loads(cargo)
    ver = t["package"]["version"]

    if ver == {"workspace": True}:
        with open("./Cargo.toml", "r") as f:
            cargo = f.read()
        t = toml.loads(cargo)
        ver = t["workspace"]["package"]["version"]

    print("--- openraft/Cargo.toml version is", ver)
    return ver


if __name__ == "__main__":
    # Usage: to build change log from git log for the current version.
    # ./scripts/build_change_log.py
    # ./scripts/build_change_log.py v0.8.3
    # ./scripts/build_change_log.py 12345abc

    new_ver = load_cargo_version()

    if len(sys.argv) == 2:
        since = sys.argv[1]
    else:
        since = None

    build_ver_changelog(new_ver, since=since)
    build_ver_changelog_summary(new_ver)
    build_changelog()
