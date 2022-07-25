#!/usr/bin/env python
# coding: utf-8

import sys
import subprocess
import semantic_version
import yaml
import re
import os
from collections import defaultdict

with open('scripts/change-types.yaml', 'r') as f:
    typs = yaml.load(f.read())

typs = {x:x for x in typs}

# categories has another mapping to fix typo in commit message
categories = {
        'api-change:':   typs['api-change'],
        'new-feature:':  typs['new-feature'],
        'internal:':     typs['internal'],
        'Doc:':          typs['doc'],
        'doc:':          typs['doc'],
        'Refactor:':     typs['refactor'],
        'refactor:':     typs['refactor'],
        'fixbug:':       typs['fixbug'],
        'fixdoc:':       typs['fixdoc'],
        'dep:':          typs['dep'],
        'Dep:':          typs['dep'],
        'ci:':           typs['ci'],
        'CI:':           typs['ci'],

        # fix typo
        'Change:':       typs['api-change'],
        'change:':       typs['api-change'],
        'changes:':      typs['api-change'],
        'api-changes:':  typs['api-change'],
        'Add:':          typs['new-feature'],
        'add:':          typs['new-feature'],
        'feature:':      typs['new-feature'],
        'Feature:':      typs['new-feature'],
        'features:':     typs['new-feature'],
        'new-features:': typs['new-feature'],
        'docs:':         typs['doc'],
        'fix:':          typs['fixbug'],
        'Fix:':          typs['fixbug'],
        'fixup:':          typs['fixbug'],

        'test:':         typs['test'],
        'build(deps):':  typs['dep'],

        'Update:':  typs['other'],
        'update:':  typs['other'],
        'turn:':  typs['other'],
        'replace:': typs['refactor'],
        'use:': typs['refactor'],
        'Create:': typs['other'],
        'BumpVer:': typs['other'], 
}

category_display = {
    "api-change": "Changed",
    "new-feature": "Added",
    "dep": "Dependency",
    "fixbug": "Fixed",
    "fixdoc": "Fixed",

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

    commits = cmd(["git", "log", '--format=%H', '--reverse', frm + '..' + to])
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
        sub = ch['subject']
        cate, cont = sub.split(' ', 1)
        catetitle = categories[cate]

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

def build_ver_changelog(new_ver, commit="HEAD"):
    '''
    Build change log for ``new_ver`` at ``commit``.
    It will find out all commit since the last tag that is less than ``new_ver``
    '''

    fn = 'change-log/v{new_ver}.md'.format(new_ver=new_ver)
    if os.path.exists(fn):
        print("--- Version {new_ver} change log exists, skip...".format(new_ver=new_ver))
        print("--- To rebuild it, delete {fn} and re-run".format(fn=fn))
        return

    tags = list_tags()
    tags.sort()

    new_ver = new_ver.lstrip('v')
    new_ver = semantic_version.Version(new_ver)
    tags = [t for t in tags if t < new_ver]
    latest = tags[-1]

    chs = changes('v' + str(latest), commit)
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

if __name__ == "__main__":
    # Usage: to build change log from git log
    # ./scripts/build_change_log.py 0.5.10
    new_ver = sys.argv[1]
    if len(sys.argv) > 2:
        commit = sys.argv[2]
    else:
        commit = 'HEAD'
    build_ver_changelog(new_ver, commit=commit)
    build_changelog()

