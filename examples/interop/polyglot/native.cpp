// C++ behind an extern "C" face: std::string does the work, the boundary
// stays the C ABI Keal binds.
#include "poly.h"
#include <string>
#include <cstdlib>
#include <cstring>

extern "C" char* cpp_shout(const char* text) {
    std::string s(text);
    for (auto& c : s) { c = (char)toupper((unsigned char)c); }
    s += "!";
    char* out = (char*)malloc(s.size() + 1);
    memcpy(out, s.c_str(), s.size() + 1);
    return out;
}
