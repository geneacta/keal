class Point:
    __slots__ = ('x','y')
    def __init__(self, x, y):
        self.x = x; self.y = y
def manhattan(p): return abs(p.x) + abs(p.y)
s = 0
for i in range(10000000):
    p = Point(i % 100 - 50, i % 37 - 18)
    s += manhattan(p)
print(s)
