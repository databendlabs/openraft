(ns jepsen.openraft.nemesis.partition-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
                    [nemesis :as nemesis]]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.nemesis.partition :as partition]))

(def nodes ["n1" "n2" "n3"])

(deftest places-leader-in-requested-component
  (testing "leader remains in the majority"
    (let [[leader-side other-side]
          (#'partition/partition-components
            nodes
            "n1"
            :leader-in-majority)]
      (is (contains? (set leader-side) "n1"))
      (is (= 2 (count leader-side)))
      (is (= 1 (count other-side)))))

  (testing "leader is isolated in the minority"
    (let [[leader-side other-side]
          (#'partition/partition-components
            nodes
            "n1"
            :leader-in-minority)]
      (is (= ["n1"] leader-side))
      (is (= #{"n2" "n3"} (set other-side))))))

(deftest resolves-leader-when-partition-starts
  (let [invocations (atom [])
        partitioner (reify nemesis/Nemesis
                      (setup! [this _test]
                        this)
                      (invoke! [_ _test op]
                        (swap! invocations conj op)
                        op)
                      (teardown! [this _test]
                        this))
        subject (partition/->PartitionNemesis partitioner)
        op {:type :info
            :process :nemesis
            :f :start-partition
            :value :leader-in-minority}]
    (with-redefs [cluster/await-ready! (fn [_test]
                                         {:leader "n2"})]
      (let [result (nemesis/invoke! subject {:nodes nodes} op)
            delegated (first @invocations)]
        (is (= :start (:f delegated)))
        (is (= #{"n2"}
               (-> result :value :components first set)))
        (is (= :leader-in-minority
               (-> result :value :mode)))
        (is (= "n2"
               (-> result :value :leader)))))))

(deftest requires-both-partitions-and-recovery
  (let [subject (#'partition/coverage-checker)
        complete-history [{:f :start-partition
                           :value {:mode :leader-in-majority}}
                          {:f :start-partition
                           :value {:mode :leader-in-minority}}
                          {:f :await-recovery
                           :value {:leader "n2"}}]
        incomplete-history [{:f :start-partition
                             :value {:mode :leader-in-majority}}]]
    (is (:valid? (checker/check subject {} complete-history {})))
    (let [result (checker/check subject {} incomplete-history {})]
      (is (false? (:valid? result)))
      (is (= [:leader-in-minority] (:missing-modes result)))
      (is (false? (:recovered? result))))))
