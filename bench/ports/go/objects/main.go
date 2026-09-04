package main

import "fmt"

type Point struct {
	x, y int64
}

func iabs(v int64) int64 {
	if v < 0 {
		return -v
	}
	return v
}

func manhattan(p Point) int64 { return iabs(p.x) + iabs(p.y) }

func main() {
	var sum int64 = 0
	for i := int64(0); i < 10000000; i++ {
		p := Point{i%100 - 50, i%37 - 18}
		sum += manhattan(p)
	}
	fmt.Println(sum)
}
