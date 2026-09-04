package main

import "fmt"

func main() {
	var total int64 = 0
	var i int64 = 0
	for i < 100000000 {
		total += i % 7
		i += 1
	}
	fmt.Println(total)
}
