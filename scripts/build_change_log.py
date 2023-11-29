#!/usr/bin/env python
# coding: utf-8

import sys
import subprocess
import semantic_version
import toml
import yaml
import re
import os
from collections import defaultdict

with open('scripts/change-types.yaml', 'r') as f:
    typs = yaml.load(f.read())

typs = {x:x for x in typs}

# categories has another mapping to fix typo in commit message
categories = {
        'data-change:':  typs['data-change'],
        'data-changes:': typs['data-change'],
        'DataChange:':   typs['data-change'],
        'DataChanges:':  typs['data-change'],

        'api-change:':   typs['api-change'],
        'new-feature:':  typs['new-feature'],
        'improve:':      typs['improve'],
        'Improve:':      typs['improve'],
        'internal:':     typs['internal'],
        'doc:':          typs['doc'],
        'Doc:':          typs['doc'],
        'refactor:':     typs['refactor'],
        'fixbug:':       typs['fixbug'],
        'fixdoc:':       typs['fixdoc'],
        'Fixdoc:':       typs['fixdoc'],
        'dep:':          typs['dep'],
        'ci:':           typs['ci'],
        'CI:':           typs['ci'],

        # fix typo
        'Change:':       typs['api-change'],
        'change:':       typs['api-change'],
        'changes:':      typs['api-change'],
        'api-changes:':  typs['api-change'],
        'Add:':          typs['new-feature'],
        'add:':          typs['new-feature'],
        'Feature:':      typs['new-feature'],
        'feature:':      typs['new-feature'],
        'Features:':     typs['new-feature'],
        'features:':     typs['new-feature'],
        'new-features:': typs['new-feature'],
        'docs:':         typs['doc'],
        'fix:':          typs['fixbug'],
        'Fix:':          typs['fixbug'],
        'fixup:':        typs['fixbug'],

        'test:':         typs['test'],
        'build(deps):':  typs['dep'],
        'Build(deps):':  typs['dep'],

        'Update:':       typs['other'],
        'update:':       typs['other'],
        'turn:':         typs['other'],
        'replace:':      typs['refactor'],
        'format:':       typs['refactor'],
        'use:':          typs['refactor'],
        'Create:':       typs['other'],
        'BumpVer:':      typs['other'],
        'Chore:':        typs['other'],
        'chore:':        typs['other'],
}

category_display = {
    "data-change": "DataChanged",
    "api-change":  "Changed",
    "new-feature": "Added",
    "improve":     "Improved",
    "dep":         "Dependency",
    "fixbug":      "Fixed",
    "fixdoc":      "DocFixed",

}

replace_subjects = [
        (r'^([^: ]+) ', r'\1: '), # add ":" if not
        (r'^(\w+:) *', r'\1 '),  # 0 or many space to 1 space
        (r'^build\(dpes\): ',  r'dep: '),
]

ignores = [
        '^Merge pull request',
        '^BumpVer:',
]

to_display = {
        'other': False,
        'doc': False,
        'refactor': False,
        'internal': False,
        'test': False,
        'ci': False,
}

commit_url_ptn = 'https://github.com/datafuselabs/openraft/commit/{hash}'

def cmd(cmds):
    subproc = subprocess.Popen(cmds,
                               encoding='utf-8',
                                  stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, )
    out, err = subproc.communicate()
    subproc.wait()

    code = subproc.returncode
    if code != 0:
        raise OSError(out + "\n" + err)

    return out

def list_tags():
    out = cmd(["git", "tag", "-l"])
    tags = out.splitlines()
    tags[0].lstrip('v')
    tags = [semantic_version.Version(t.lstrip('v'))
            for t in tags
            if t != '' and t != 'base' ]
    return tags


def changes(frm, to):
    # subject, author time, author name, email.

    git_cmd = ["git", 'merge-base', frm, to]
    common_base = cmd(git_cmd).strip()
    print("--- Find common base {common_base} of {frm} {to} with: {git_cmd}".format(
            common_base=common_base,
            frm=frm,
            to=to,
            git_cmd=git_cmd))

    git_cmd = ["git", "log", '--format=%H', '--reverse', common_base + '..' + to]
    print("--- List commits with: {git_cmd}".format(git_cmd=' '.join(git_cmd)))

    commits = cmd(git_cmd)
    commits = commits.splitlines()

    rst = []

    for commit in commits:
        # Extract subject, date, author and email
        out = cmd(["git", "log", '-1', '--format=%s ||| %ai ||| %an ||| %ae', commit])
        line = out.strip()

        if if_ignore(line):
            continue

        line = replace_subject(line)

        elts = line.split(" ||| ")

        body = cmd(["git", "log", '-1', '--format=%b', commit])

        item = {
                'hash': commit,
                'subject': elts[0],
                # 2019-04-18 13:36:42 +0800
                'time': elts[1].split()[0],
                'author': elts[2],
                'email': elts[3],
                'body': body,
        }

        rst.append(item)

    return rst

def replace_subject(line):
    output = line
    for (ptn, repl) in replace_subjects:
        output = re.sub(ptn, repl, output)

    if output != line:
        print("--- Fix commit message, replace", ptn, 'with', repl)
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
    '''
    Collect changes by category.
    Indent commit message body.
    '''
    rst = {}
    for ch in changes:
        print("--- Normalize subject:", ch['hash'], ch['subject'])
        sub = ch['subject']
        cate, cont = (sub.split(' ', 1) + [''])[:2]
        catetitle = categories.get(cate, typs['other'])

        if catetitle not in rst:
            rst[catetitle] = []

        c = rst[catetitle]
        bodylines = ch['body'].strip().splitlines()
        bodylines = ['    ' + x for x in bodylines]
        desc = {
                'hash': ch['hash'],
                "content": cont,
                "time": ch['time'],
                "author": ch['author'],
                'email': ch['email'],
                'body': bodylines,
        }
        c.append(desc)

    return rst

def build_ver_changelog(new_ver, commit="HEAD", since=None):
    '''
    Build change log for ``new_ver`` at ``commit``.
    It will find out all commit since the last tag that is less than ``new_ver``

    If ``since`` is specified, build change log since ``since`` upto ``commit``.
    '''

    fn  = version_chaagelog_fn(new_ver)
    if os.path.exists(fn):
        print("--- Version {new_ver} change log exists, skip...".format(new_ver=new_ver))
        print("--- To rebuild it, delete {fn} and re-run".format(fn=fn))
        return

    if since is None:
        tags = list_tags()
        tags.sort()

        new_ver = new_ver.lstrip('v')
        new_ver = semantic_version.Version(new_ver)
        tags = [t for t in tags if t < new_ver]
        latest = 'v' + str(tags[-1])
    else:
        latest = since

    chs = changes(latest, commit)
    chs = norm_changes(chs)

    lines = []
    for cate, descs in chs.items():
        if not to_display.get(cate, True):
            continue

        cate = category_display[cate]

        cate_title = '### {cate}:'.format(cate=cate)
        lines.append(cate_title)
        lines.append("")

        for desc in descs:
            short_hash = desc['hash'][:8]
            url = commit_url_ptn.format(**desc)

            title = '-   {cate}: [{short_hash}]({url}) {content}; by {author}; {time}'.format(cate=cate, short_hash=short_hash, url=url, **desc)
            lines.append(title)
            lines.append('')
            if len(desc['body']) > 0:
                lines.extend(desc['body'])
                lines.append('')

    # remove blank
    lines = [x.rstrip() for x in lines]

    changelog = '\n'.join(lines)

    with open(fn, 'w') as f:
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

    with open(fn, 'r') as f:
        lines = f.readlines()

    lines = [l.rstrip() for l in lines]

    # Remove existent summary
    try:
        footer_indexs = lines.index(footer)
    except ValueError as e:
        print("No footer found")
    else:
        # skip `Detail:`, and a following blank line.
        lines = lines[footer_indexs+2:]

    # Build summary:
    # - Remove lines that starts with space, which is detail section, a list item
    #   with indent.
    # - Remove blank lines
    source = [l for l in lines if not l.startswith(" ") and l != ""]
    summary = [header, ""]
    for l in source:
        if l.startswith("### "):
            # Section header, Strip "### "
            l = "- " + l[4:]

        elif l.startswith('-   '):
            # Log:
            #   remove `Fixed:`,
            l = l.split(sep=None, maxsplit=2)[2]

            # First letter upper case
            if l[0] >= 'a' and l[0] <= 'z':
                l = l[0].upper() + l[1:]

            # Remove suffix date
            l = re.sub('; \d\d\d\d-\d\d-\d\d$', '', l)

            # Remove suffix author
            l = re.sub('; by .*?$', '.', l)

            # Remove redundent "."
            l = re.sub('[.][.]$', '.', l)

            #   add indent:
            l = "    -   " + l
        else:
            raise ValueError("unknown line: " + repr(l))

        summary.append(l)

    summary.append("")
    summary.append(footer)
    summary.append("")

    with open(fn, 'w') as f:
        for l in summary:
            f.write(l + "\n")
        for l in lines:
            f.write(l + "\n")


def version_chaagelog_fn(ver):
    _ = semantic_version.Version(ver)

    fn = 'change-log/v{ver}.md'.format(ver=ver)

    return fn


def build_changelog():

    out = cmd(["ls", "change-log"])
    vers = out.splitlines()
    # remove suffix "md"
    vers = [x.rsplit('.', 1)[0] for x in vers if x != '']
    vers.sort(key=lambda x: semantic_version.Version(x.lstrip('v')))

    with open('change-log.md', 'w') as f:
        for v in reversed(vers):
            print("--- append change log of {v}".format(v=v))

            f.write("## " + v + '\n\n')
            with open('change-log/{v}.md'.format(v=v), 'r') as vf:
                cont = vf.read()

            cont = cont.splitlines()
            cont = '\n'.join(cont)

            f.write(cont + '\n\n')

def load_cargo_version():
    with open('./openraft/Cargo.toml',  'r') as f:
        cargo = f.read()

    t = toml.loads(cargo)
    ver = t['package']['version']

    if ver == {'workspace': True}:
        with open('./Cargo.toml',  'r') as f:
            cargo = f.read()
        t = toml.loads(cargo)
        ver = t['workspace']['package']['version']

    print("--- openraft/Cargo.toml version is",  ver)
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

