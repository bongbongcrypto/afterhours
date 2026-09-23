# -*- coding: utf-8 -*-
"""Check that the hand-written integrator interface (abi/IAfterHours.sol)
declares exactly the functions, return types and errors the contract exports
(`cargo stylus export-abi`). Parameter names and comments are ignored; types,
order and mutability are compared. Stdlib only; CI runs it after export-abi.

    python scripts/abi_check.py out/AfterHours.abi.sol abi/IAfterHours.sol
"""
import re
import sys


def declarations(path):
    text = open(path, encoding="utf-8").read()
    text = re.sub(r"//[^\n]*", "", text)                     # line and NatSpec comments
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    body = text[text.index("{", text.index("interface")) + 1: text.rindex("}")]
    out = set()
    for stmt in (" ".join(s.split()) for s in body.split(";")):
        m = re.match(r"function (\w+)\s*\(([^)]*)\)(.*)", stmt)
        if m:
            name, params, rest = m.groups()
            ret = re.search(r"returns\s*\(([^)]*)\)", rest)
            mut = "view" if re.search(r"\bview\b", rest) else "nonpayable"
            out.add("function %s(%s) %s returns (%s)" % (name, types(params), mut, types(ret.group(1)) if ret else ""))
            continue
        m = re.match(r"error (\w+)\s*\(([^)]*)\)", stmt)
        if m:
            out.add("error %s(%s)" % (m.group(1), types(m.group(2))))
    return out


def types(params):
    # "address[] memory pools" -> "address[]"; "uint8" -> "uint8"
    return ",".join(p.split()[0] for p in params.split(",") if p.strip())


def main():
    exported, written = declarations(sys.argv[1]), declarations(sys.argv[2])
    missing, extra = sorted(exported - written), sorted(written - exported)
    for d in missing:
        print("exported by the contract, missing or different in %s: %s" % (sys.argv[2], d))
    for d in extra:
        print("in %s but not exported by the contract: %s" % (sys.argv[2], d))
    if missing or extra:
        sys.exit(1)
    print("%s matches the exported ABI: %d declarations" % (sys.argv[2], len(exported)))


if __name__ == "__main__":
    main()
