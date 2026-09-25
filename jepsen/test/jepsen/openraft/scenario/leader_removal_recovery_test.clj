(ns jepsen.openraft.scenario.leader-removal-recovery-test
  (:require [clojure.java.shell :as shell]
            [clojure.string :as str]
            [jepsen.control :as c]
            [clojure.test :refer [deftest is testing]]
            [jepsen.generator.test :as gen-test]
            [jepsen.openraft.client :as http]
            [jepsen.openraft.harness :as harness]
            [jepsen.openraft.scenario.leader-removal-recovery :as sut]))

(def nodes ["n1" "n2"])
(def joint {:log_id {:leader_id {:term 1 :node_id "n1"} :index 3}
            :membership {:configs [nodes ["n2"]] :nodes {:n1 {} :n2 {}}}})
(def final-membership (-> joint
                          (assoc-in [:log_id :index] 4)
                          (assoc-in [:membership :configs] [["n2"]])))
(def window [{:membership_config final-membership :committed_membership_config joint
              :local_committed {:index 3} :last_applied {:index 3}}
             {:membership_config joint :committed_membership_config joint
              :last_log_index 3 :last_applied {:index 3}}])
(def recovered {:status :recovered :elapsed-ms 1000 :fresh "after-heal" :sentinel "before-crash"
                :metrics {:state "Leader" :current_leader "n2" :last_applied {:index 5}
                          :membership_config final-membership
                          :committed_membership_config final-membership}})
(def history (mapv (fn [i f value] {:process :nemesis :type :info :f f :time i :value value})
                   (range)
                   [:prepare-leader-removal :restart-removed-leader :heal-leader-removal :check-leader-removal]
                   [{:status :prepared :metrics window} {:status :restarted} {:status :healed} recovered]))

(deftest rejects-missing-or-wrong-evidence
  (let [valid? #(:valid? (sut/verdict {:nodes nodes} %))]
    (is (true? (valid? history)))
    (testing "Nemesis invocations and completions both have type info"
      (is (true? (valid? (mapcat #(vector (assoc % :value nil) %) history)))))
    (doseq [i (range 4)]
      (is (false? (valid? (vec (concat (take i history) (drop (inc i) history)))))))
    (doseq [[path value] [[[0 :value :metrics 1 :last_log_index] 4]
                          [[0 :value :metrics 0 :local_committed :index] 4]
                          [[0 :value :metrics 1 :last_applied :index] 2]
                          [[3 :value :status] :timeout]
                          [[3 :value :elapsed-ms] 60001]
                          [[3 :value :sentinel] nil]
                          [[3 :value :fresh] "wrong"]
                          [[3 :value :metrics :current_leader] "n1"]
                          [[3 :value :metrics :last_applied :index] 3]
                          [[3 :time] 1]]]
      (is (false? (valid? (assoc-in history path value))) (str path)))
    (testing "recreating the same voter set at a different log ID cannot pass"
      (let [other (assoc-in final-membership [:log_id :index] 6)]
        (is (false? (valid? (-> history
                                (assoc-in [3 :value :metrics :membership_config] other)
                                (assoc-in [3 :value :metrics :committed_membership_config] other)
                                (assoc-in [3 :value :metrics :last_applied :index] 7)))))))))

(deftest partition-is-installed-before-final-is-submitted
  (let [calls (atom [])]
    (with-redefs [http/write! (fn [& _] (swap! calls conj :sentinel))
                  http/append-membership! (fn [_ configs]
                                            (swap! calls conj configs)
                                            (when (= configs [["n2"]])
                                              (throw (ex-info "uncommitted" {:kind :request-timeout}))))
                  sut/await-state! (fn [_ condition _] (swap! calls conj condition) window)
                  sut/partition! (fn [_] (swap! calls conj :partition))]
      (is (= window (#'sut/prepare! {:nodes nodes}))))
    (is (= [:sentinel [nodes ["n2"]] :joint-applied :partition [["n2"]] :final-window] @calls))))

(deftest partition-install-minimizes-the-leader-lease-critical-path
  (let [commands (atom [])
        script (str "iptables -w -N OR_LEADER_REMOVAL && "
                    "iptables -w -A OR_LEADER_REMOVAL -p tcp --dport 22099 -j DROP && "
                    "iptables -w -C OR_LEADER_REMOVAL -p tcp --dport 22099 -j DROP && "
                    "iptables -w -I OUTPUT -j OR_LEADER_REMOVAL && "
                    "iptables -w -C OUTPUT -j OR_LEADER_REMOVAL")]
    (with-redefs [c/on-nodes (fn [test f]
                               (doseq [node (:nodes test)] (f test node)))
                  c/exec (fn [& command] (swap! commands conj command))]
      (#'sut/partition! {:nodes nodes :raft-port 22099}))
    (is (= [[:bash :-ceu script] [:bash :-ceu script]] @commands))))

(deftest cleanup-survives-a-harness-failure
  (let [failure (harness/failure-state)]
    (harness/record-failure! failure :nemesis {} (ex-info "failed setup" {}))
    (is (= [:heal-leader-removal]
           (mapv :f (filter #(= :ok (:type %))
                            (gen-test/perfect* (sut/generator failure))))))))

(deftest cleanup-propagates-iptables-failures
  (let [failure (ex-info "iptables failed" {:exit 4})]
    (with-redefs [c/on-nodes (fn [test f] (f test "n1"))
                  c/exec (fn [& command]
                           ;; Exercise shell error handling too, so the old conditional
                           ;; cannot turn the simulated iptables failure into success.
                           (let [script (if (= :bash (first command))
                                          (last command)
                                          (str/join " " (map name command)))
                                 result (shell/sh "bash" "-ceu"
                                                  (str "iptables() { return 4; }\n" script))]
                             (if (zero? (:exit result)) (:out result) (throw failure))))]
      (is (identical? failure (try (#'sut/heal! {:nodes nodes})
                                   (catch Exception e e)))))))

(deftest cleanup-only-removes-its-own-chain
  (doseq [present? [false true]]
    (let [commands (atom [])
          chain "OR_LEADER_REMOVAL"
          rules (str "-P OUTPUT ACCEPT\n-N OTHER\n-A OUTPUT -j OTHER\n"
                     (when present? (str "-N " chain "\n-A OUTPUT -j " chain "\n")))]
      (with-redefs [c/on-nodes (fn [test f] (f test "n1"))
                    c/exec (fn [& command] (swap! commands conj (vec command)) rules)]
        (#'sut/heal! {:nodes nodes}))
      (is (= (cond-> [[:iptables :-w :-S]]
               present? (into [[:iptables :-w :-D :OUTPUT :-j chain]
                               [:iptables :-w :-F chain]
                               [:iptables :-w :-X chain]]))
             @commands)))))
