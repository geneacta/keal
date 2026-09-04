xs = []
for i in range(1000000): xs.append(i % 1000)
doubled = list(map(lambda it: it * 2, xs))
big = list(filter(lambda it: it > 1000, doubled))
from functools import reduce
print(reduce(lambda acc, n: acc + n, big, 0))
