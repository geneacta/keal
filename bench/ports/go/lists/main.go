package main

import "fmt"

func main() {
	xs := []int64{}
	for i := int64(0); i < 1000000; i++ {
		xs = append(xs, i%1000)
	}

	doubled := []int64{}
	for _, v := range xs {
		doubled = append(doubled, v*2)
	}

	big := []int64{}
	for _, v := range doubled {
		if v > 1000 {
			big = append(big, v)
		}
	}

	var acc int64 = 0
	for _, v := range big {
		acc = acc + v
	}
	fmt.Println(acc)
}
