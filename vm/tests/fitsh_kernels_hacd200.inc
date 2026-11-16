// batch/single HACD scorer, unified one-record-per-iteration loop
var cnt = 1u64
if size(data) != 32 {
    cnt = byte(data, 0) * 256 + byte(data, 1)
}
var out = ""
var r = 0u64
while r < cnt {
    var nb = 0
    var gb = 0
    var num = 0u64
    var wide = 0
    if size(data) == 32 {
        nb = 0
        gb = 6
        var j = 0u64
        while j < 8 { num = num * 256 + byte(data, 24 + j)
        j += 1 }
        wide = 1
    } else {
        nb = 2 + r * 20
        gb = nb + 6
        var j2 = 0u64
        while j2 < 4 { num = num * 256 + byte(data, nb + 16 + j2)
        j2 += 1 }
        wide = 0
    }
    var shape = 0u64
    var sa = byte(data, gb) / 16
    var sb = byte(data, gb) % 16
    if sa == 0 {
        if sb == 1 { shape = 30 }
        if sb == 2 { shape = 30 }
        if sb == 3 { shape = 50 }
        if sb == 4 { shape = 20 }
        if sb == 5 { shape = 30 }
        if sb == 6 { shape = 30 }
        if sb == 7 { shape = 20 }
        if sb == 8 { shape = 20 }
    }
    var colorish = 0u64
    if shape == 0 {
        var c13 = byte(data, gb + 7) / 16
        var c14 = byte(data, gb + 7) % 16
        var c15 = byte(data, gb + 8) / 16
        var c16 = byte(data, gb + 8) % 16
        var pos = 0u64
        if c13 == c14 && c14 == c15 && c15 == c16 { pos = 250 } else {
        if c13 == c14 && c14 == c15 && c14 != c16 { pos = 20 } else {
        if c13 == c15 && c15 == c16 && c14 != c13 { pos = 20 } else {
        if c13 == c14 && c14 == c16 && c15 != c13 { pos = 20 } else {
        if c14 == c15 && c15 == c16 && c14 != c13 { pos = 20 } else {
        if c16 == c15 && c13 == c14 { pos = 20 } else {
        if c16 == c14 && c13 == c15 { pos = 20 } else {
        if c15 * 16 + c13 == c14 * 16 + c16 { pos = 20 } else {
        if c13 == c14 && c15 != c16 { pos = 3 } else {
        if c15 == c16 && c14 != c13 { pos = 3 }
        }}}}}}}}}
        var alldark = 1u64
        var alllight = 1u64
        var bm3 = 0u64
        var k = 2u64
        while k < 18 {
            var hv = byte(data, gb + k / 2)
            if k % 2 == 0 { hv = hv / 16 } else { hv = hv % 16 }
            bm3 = bm3 | (1u64 << hv)
            if hv > 4 { alldark = 0 }
            if hv <= 4 { alllight = 0 }
            k += 1
        }
        var px3 = bm3 & 65535
        px3 = px3 - ((px3 >> 1) & 21845)
        px3 = (px3 & 13107) + ((px3 >> 2) & 13107)
        px3 = (px3 + (px3 >> 4)) & 3855
        px3 = px3 + (px3 >> 8)
        var distinct3 = px3 & 31
        var colcnt = 0u64
        if distinct3 == 4 { colcnt = 200 }
        if distinct3 == 5 { colcnt = 100 }
        if distinct3 == 6 { colcnt = 50 }
        if distinct3 == 7 { colcnt = 20 }
        if distinct3 == 14 { colcnt = 30 }
        if distinct3 == 15 { colcnt = 100 }
        if distinct3 == 16 { colcnt = 200 }
        colorish = pos
        if colcnt > colorish { colorish = colcnt }
        if alldark == 1 { if 20 > colorish { colorish = 20 } }
        if alllight == 1 { if 20 > colorish { colorish = 20 } }
    } else {
        var nc = byte(data, nb)
        var cs = 0u64
        if nc == 65 { cs = 7 }
        if nc == 66 { cs = 6 }
        if nc == 69 { cs = 1 }
        if nc == 72 { cs = 8 }
        if nc == 73 { cs = 8 }
        if nc == 75 { cs = 8 }
        if nc == 77 { cs = 2 }
        if nc == 78 { cs = 6 }
        if nc == 83 { cs = 6 }
        if nc == 84 { cs = 8 }
        if nc == 85 { cs = 3 }
        if nc == 86 { cs = 8 }
        if nc == 87 { cs = 3 }
        if nc == 88 { cs = 5 }
        if nc == 89 { cs = 3 }
        if nc == 90 { cs = 10 }
        var alldark2 = 1u64
        var alllight2 = 1u64
        var k2 = 2u64
        while k2 < 19 {
            var hv2 = byte(data, gb + k2 / 2)
            if k2 % 2 == 0 { hv2 = hv2 / 16 } else { hv2 = hv2 % 16 }
            if hv2 > 4 { alldark2 = 0 }
            if hv2 <= 4 { alllight2 = 0 }
            k2 += 1
        }
        colorish = cs
        if alldark2 == 1 { if 20 > colorish { colorish = 20 } }
        if alllight2 == 1 { if 20 > colorish { colorish = 20 } }
    }
    var maxrun = 1u64
    var run = 1u64
    var q = 1u64
    while q < 6 {
        var c1 = byte(data, nb + q)
        var c0 = byte(data, nb + q - 1)
        if c1 == c0 {
            run += 1
            if run > maxrun { maxrun = run }
        } else {
            run = 1
        }
        q += 1
    }
    var nl = 0u64
    if maxrun >= 3 {
        if maxrun == 6 { nl = 200 }
        if maxrun == 5 { if 50 > nl { nl = 50 } }
        if maxrun == 4 { if 10 > nl { nl = 10 } }
        if maxrun < 4 { if 2 > nl { nl = 2 } }
    }
    var bm2 = 0u64
    q = 0
    while q < 6 {
        var lc = byte(data, nb + q)
        var li = 16u64
        if lc == 87 { li = 0 }
        if lc == 84 { li = 1 }
        if lc == 89 { li = 2 }
        if lc == 85 { li = 3 }
        if lc == 73 { li = 4 }
        if lc == 65 { li = 5 }
        if lc == 72 { li = 6 }
        if lc == 88 { li = 7 }
        if lc == 86 { li = 8 }
        if lc == 77 { li = 9 }
        if lc == 69 { li = 10 }
        if lc == 75 { li = 11 }
        if lc == 66 { li = 12 }
        if lc == 83 { li = 13 }
        if lc == 90 { li = 14 }
        if lc == 78 { li = 15 }
        bm2 = bm2 | (1u64 << li)
        q += 1
    }
    var px2 = bm2 & 65535
    px2 = px2 - ((px2 >> 1) & 21845)
    px2 = (px2 & 13107) + ((px2 >> 2) & 13107)
    px2 = (px2 + (px2 >> 4)) & 3855
    px2 = px2 + (px2 >> 8)
    var distinct2 = px2 & 31
    if distinct2 == 2 { if 10 > nl { nl = 10 } }
    if distinct2 == 3 { if 3 > nl { nl = 3 } }
    if byte(data, nb) == byte(data, nb + 3) && byte(data, nb + 1) == byte(data, nb + 4) && byte(data, nb + 2) == byte(data, nb + 5) {
        if 20 > nl { nl = 20 }
    }
    if byte(data, nb) == byte(data, nb + 5) && byte(data, nb + 1) == byte(data, nb + 4) && byte(data, nb + 2) == byte(data, nb + 3) {
        if 10 > nl { nl = 10 }
    }
    if byte(data, nb) == byte(data, nb + 1) && byte(data, nb + 3) == byte(data, nb + 4) {
        if 10 > nl { nl = 10 }
    }
    if byte(data, nb + 1) == byte(data, nb + 2) && byte(data, nb + 4) == byte(data, nb + 5) {
        if 10 > nl { nl = 10 }
    }
    var nu = 0u64
    if num < 10 { nu = 30 }
    var digs = ""
    var x = num
    if x == 0 { digs = "0" }
    while x > 0 {
        var d10 = x % 10
        var ch = "9"
        if d10 == 0 { ch = "0" }
        if d10 == 1 { ch = "1" }
        if d10 == 2 { ch = "2" }
        if d10 == 3 { ch = "3" }
        if d10 == 4 { ch = "4" }
        if d10 == 5 { ch = "5" }
        if d10 == 6 { ch = "6" }
        if d10 == 7 { ch = "7" }
        if d10 == 8 { ch = "8" }
        digs = ch ++ digs
        x = x / 10
    }
    var dn = size(digs)
    var same = 1u64
    var j3 = 1u64
    while j3 < dn {
        if byte(digs, j3) != byte(digs, 0) { same = 0 }
        j3 += 1
    }
    if same == 1 { if 50 > nu { nu = 50 } }
    var up = 1u64
    var down = 1u64
    var j4 = 1u64
    while j4 < dn {
        var cur = byte(digs, j4)
        var pre = byte(digs, j4 - 1)
        if cur <= pre { up = 0 } else { if cur - pre != 1 { up = 0 } }
        if cur >= pre { down = 0 } else { if pre - cur != 1 { down = 0 } }
        j4 += 1
    }
    if up == 1 || down == 1 { if 25 > nu { nu = 25 } }
    var sym = 1u64
    var j5 = 0u64
    while j5 < dn / 2 {
        if byte(digs, j5) != byte(digs, dn - 1 - j5) { sym = 0 }
        j5 += 1
    }
    if sym == 1 { if 10 > nu { nu = 10 } }
    var mrun = 1u64
    run = 1
    var j6 = 1u64
    while j6 < dn {
        if byte(digs, j6) == byte(digs, j6 - 1) {
            run += 1
            if run > mrun { mrun = run }
        } else {
            run = 1
        }
        j6 += 1
    }
    if mrun >= 4 {
        if mrun == 7 { nu = 200 }
        if mrun == 6 { if 100 > nu { nu = 100 } }
        if mrun == 5 { if 50 > nu { nu = 50 } }
        if mrun < 5 { if 20 > nu { nu = 20 } }
        if mrun > 7 { if 20 > nu { nu = 20 } }
    }
    var total = shape + colorish + nl + nu
    if total < 1 { total = 1 }
    var w8 = total as bytes
    if wide == 1 {
        out = out ++ w8
    } else {
        out = out ++ buf_cut(w8, 6, 2)
    }
    r += 1
}
return out
