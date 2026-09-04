import java.util.ArrayList;
import java.util.List;
import java.util.stream.Collectors;

public class Lists {
    public static void main(String[] args) {
        List<Long> xs = new ArrayList<>();
        for (long i = 0; i < 1000000L; i++) xs.add(i % 1000);

        List<Long> doubled = xs.stream().map(it -> it * 2).collect(Collectors.toList());
        List<Long> big = doubled.stream().filter(it -> it > 1000).collect(Collectors.toList());

        long acc = 0;
        for (long n : big) acc = acc + n;
        System.out.println(acc);
    }
}
