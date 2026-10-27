from PIL import Image

def sel_bbox(path):
    im = Image.open(path).convert("RGB")
    W,H = im.size
    px = im.load()
    block=[]; band=[]
    for y in range(H):
        for x in range(W):
            r,g,b = px[x,y]
            # teal selection family
            if 205<=r<=236 and 238<=g<=255 and 230<=b<=252:
                if r <= 221:
                    band.append((x,y))
                else:
                    block.append((x,y))
    def bb(pts):
        if not pts: return None
        xs=[p[0] for p in pts]; ys=[p[1] for p in pts]
        return (min(xs),min(ys),max(xs),max(ys),len(pts))
    return bb(block), bb(band), (W,H)

for p,l in [
    (r"D:/Projektdateien/Monet/.workbuddy-ai/shots/v_lvl2.png","LVL2 Color/Color (level2, children)"),
    (r"D:/Projektdateien/Monet/.workbuddy-ai/shots/v_all.png","ALL variables (single row)"),
    (r"D:/Projektdateien/Monet/.workbuddy-ai/shots/v_group.png","LVL1 Color (children)"),
]:
    bk,bd,size = sel_bbox(p)
    print(f"{l}  ({size[0]}x{size[1]})")
    if bk: print(f"  BLOCK  x[{bk[0]}..{bk[2]}] y[{bk[1]}..{bk[3]}]  L={bk[0]} R={bk[2]} top={bk[1]} bot={bk[3]} px={bk[4]}")
    if bd: print(f"  BAND   x[{bd[0]}..{bd[2]}] y[{bd[1]}..{bd[3]}]  L={bd[0]} R={bd[2]} top={bd[1]} bot={bd[3]} px={bd[4]}")
    if bk and bd:
        print(f"  -> band.left - block.left = {bd[0]-bk[0]}   band.right - block.right = {bd[2]-bk[2]}")
        print(f"  -> block bottom-to-image-bottom unhelpful; block bottom={bk[3]}, band bottom={bd[3]}")
    print()
