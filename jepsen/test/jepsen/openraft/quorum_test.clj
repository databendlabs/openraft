(ns jepsen.openraft.quorum-test
  (:require [clojure.set :as set]
            [clojure.test :refer [deftest is]]
            [jepsen.openraft.quorum :as quorum]))

(deftest enumerates-stable-membership-quorums
  (let [configs [#{"a" "b" "c"}]]
    (is (= #{#{"a" "b"}
             #{"a" "c"}
             #{"b" "c"}
             #{"a" "b" "c"}}
           (set (quorum/quorum-sets configs))))))

(deftest enumerates-joint-membership-quorums
  (let [configs [#{"a" "b" "c"}
                 #{"a" "b" "d"}]]
    (is (= #{#{"a" "b"}
             #{"a" "b" "c"}
             #{"a" "b" "d"}
             #{"a" "c" "d"}
             #{"b" "c" "d"}
             #{"a" "b" "c" "d"}}
           (set (quorum/quorum-sets configs))))))

(deftest fault-sets-leave-a-quorum-alive
  (let [configs [#{"a" "b" "c"}
                 #{"a" "b" "d"}]
        voters (quorum/voter-set configs)
        faults (quorum/fault-sets configs)]
    (is (= #{#{"a"}
             #{"b"}
             #{"c"}
             #{"d"}
             #{"c" "d"}}
           (set faults)))
    (doseq [fault faults]
      (is (quorum/quorum? configs
                          (set/difference voters fault))))))
