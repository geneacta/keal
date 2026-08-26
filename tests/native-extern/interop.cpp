#include <cstdint>
#include <vector>

// C++ freely on the inside, C linkage at the boundary.
extern "C" int64_t fib_cpp(int64_t n) {
    std::vector<int64_t> memo = {0, 1};
    for (int64_t i = 2; i <= n; i++) {
        memo.push_back(memo[i - 1] + memo[i - 2]);
    }
    return n < (int64_t)memo.size() ? memo[n] : 0;
}
