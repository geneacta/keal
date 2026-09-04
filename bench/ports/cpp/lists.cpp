#include <cstdio>
#include <cstdint>
#include <vector>
#include <algorithm>
#include <numeric>
int main() {
    std::vector<std::int64_t> xs;
    for (std::int64_t i = 0; i < 1000000; i++) xs.push_back(i % 1000);

    std::vector<std::int64_t> doubled;
    std::transform(xs.begin(), xs.end(), std::back_inserter(doubled),
                   [](std::int64_t v) { return v * 2; });

    std::vector<std::int64_t> big;
    std::copy_if(doubled.begin(), doubled.end(), std::back_inserter(big),
                 [](std::int64_t v) { return v > 1000; });

    std::int64_t acc = std::accumulate(big.begin(), big.end(), (std::int64_t)0,
                                       [](std::int64_t a, std::int64_t n) { return a + n; });
    std::printf("%lld\n", (long long)acc);
    return 0;
}
