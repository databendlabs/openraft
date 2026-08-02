(ns jepsen.openraft.nemesis.membership-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
             [nemesis :as nemesis]]
            [jepsen.openraft [client :as client]
             [cluster :as cluster]
             [db :as openraft-db]]
            [jepsen.openraft.nemesis.membership :as membership]
            [jepsen.random :as random]))

(def nodes ["n1" "n2" "n3" "n4" "n5"])

(def test-config
  {:nodes nodes
   :api-port 21001
   :raft-port 22001})

(defn- membership-nemesis []
  (membership/->MembershipNemesis :database (atom nil)))

(defn- stable-status [voters]
  {:leader "n1"
   :voters voters
   :learners #{}
   :non-members (set (remove voters nodes))
   :stable? true
   :metrics (zipmap voters (repeat {}))})

(deftest grows-without-waiting-for-stable-membership
  (let [calls (atom [])
        before #{"n1" "n2" "n3" "n4"}
        after (conj before "n5")
        statuses (atom [(stable-status before)
                        (update (stable-status before)
                                :metrics
                                assoc
                                "n5"
                                {})])]
    (with-redefs [cluster/membership-status
                  (fn [_test]
                    (let [status (first @statuses)]
                      (swap! statuses subvec 1)
                      status))
                  cluster/await-stable-membership!
                  (fn [& _]
                    (throw (ex-info "runtime grow must not wait" {})))
                  openraft-db/start-empty-node-without-wait!
                  (fn [database _test node]
                    (swap! calls conj [:start-empty database node]))
                  client/add-learner!
                  (fn [endpoint node-id api-addr raft-addr]
                    (swap! calls conj
                           [:add-learner endpoint node-id api-addr raft-addr]))
                  client/change-membership!
                  (fn [endpoint node-ids]
                    (swap! calls conj
                           [:change-membership endpoint node-ids]))]
      (let [subject (membership-nemesis)
            starting (nemesis/invoke! subject
                                      test-config
                                      {:type :info
                                       :f :grow})
            result (nemesis/invoke! subject
                                    test-config
                                    {:type :info
                                     :f :grow})]
        (is (= :node-starting (get-in starting [:value :status])))
        (is (= [[:start-empty :database "n5"]
                [:add-learner
                 "n1:21001"
                 "n5"
                 "n5:21001"
                 "n5:22001"]
                [:change-membership
                 "n1:21001"
                 ["n1" "n2" "n3" "n4" "n5"]]]
               @calls))
        (is (= {:change :grow
                :node "n5"
                :source :non-member
                :before before
                :after after}
               (select-keys (:value result)
                            [:change :node :source :before :after])))))))

(deftest handles-an-existing-learner
  (let [before #{"n1" "n2" "n3" "n4"}
        after (conj before "n5")
        calls (atom [])
        reachable (assoc (stable-status before)
                         :learners #{"n5"}
                         :non-members #{}
                         :metrics {"n1" {} "n2" {} "n3" {}
                                   "n4" {} "n5" {}})
        unreachable (update reachable :metrics dissoc "n5")]
    (testing "a reachable learner is promoted without resetting its data"
      (with-redefs [cluster/membership-status (constantly reachable)
                    openraft-db/start-empty-node!
                    (fn [& _]
                      (throw (ex-info "MUST NOT reset a learner" {})))
                    client/add-learner!
                    (fn [& _]
                      (swap! calls conj :add-learner))
                    client/change-membership!
                    (fn [& _]
                      (swap! calls conj :change-membership))]
        (let [result (nemesis/invoke! (membership-nemesis)
                                      test-config
                                      {:type :info
                                       :f :grow})]
          (is (= [:add-learner :change-membership] @calls))
          (is (= after (get-in result [:value :after]))))))

    (testing "an unreachable learner is reported without blocking"
      (reset! calls [])
      (with-redefs [cluster/membership-status (constantly unreachable)
                    client/add-learner!
                    (fn [& _]
                      (swap! calls conj :add-learner))]
        (let [result (nemesis/invoke! (membership-nemesis)
                                      test-config
                                      {:type :info
                                       :f :grow})]
          (is (= :learner-unreachable
                 (get-in result [:value :status])))
          (is (empty? @calls)))))))

(deftest defers-an-indeterminate-learner-addition
  (let [before #{"n1" "n2" "n3" "n4"}
        changed? (atom false)
        status (update (stable-status before)
                       :metrics
                       assoc
                       "n5"
                       {})]
    (with-redefs [cluster/membership-status
                  (constantly status)
                  client/add-learner!
                  (fn [& _]
                    (throw (ex-info "timeout"
                                    {:kind :request-timeout})))
                  client/change-membership!
                  (fn [& _]
                    (reset! changed? true))]
      (let [result (nemesis/invoke! (membership-nemesis)
                                    test-config
                                    {:type :info
                                     :f :grow})]
        (is (= :indeterminate (get-in result [:value :status])))
        (is (false? @changed?))))))

(deftest confirms-an-indeterminate-grow-from-membership-state
  (let [before #{"n1" "n2" "n3" "n4"}
        after (conj before "n5")
        statuses (atom [(update (stable-status before)
                                :metrics
                                assoc
                                "n5"
                                {})
                        (stable-status after)])
        change-attempts (atom 0)
        subject (membership-nemesis)]
    (with-redefs [cluster/membership-status
                  (fn [_test]
                    (let [status (first @statuses)]
                      (swap! statuses subvec 1)
                      status))
                  client/add-learner! (fn [& _])
                  client/change-membership!
                  (fn [& _]
                    (swap! change-attempts inc)
                    (throw (ex-info "timeout"
                                    {:kind :request-timeout})))]
      (let [pending (nemesis/invoke! subject
                                     test-config
                                     {:type :info
                                      :f :grow})]
        (is (= :indeterminate (get-in pending [:value :status]))))

      (let [resolved (nemesis/invoke! subject
                                      test-config
                                      {:type :info
                                       :f :shrink})]
        (is (= 1 @change-attempts))
        (is (= {:change :grow
                :before before
                :after after}
               (select-keys (:value resolved)
                            [:change :before :after])))))))

(deftest shrinks-without-waiting-before-cleanup
  (let [calls (atom [])
        before (set nodes)
        after (disj before "n5")]
    (with-redefs [cluster/membership-status
                  (constantly (stable-status before))
                  cluster/await-stable-membership!
                  (fn [& _]
                    (throw (ex-info "runtime shrink must not wait" {})))
                  random/shuffle reverse
                  client/change-membership!
                  (fn [endpoint node-ids]
                    (swap! calls conj
                           [:change-membership endpoint node-ids]))
                  openraft-db/stop-and-wipe-node!
                  (fn [database _test node]
                    (swap! calls conj [:stop-and-wipe database node]))]
      (let [result (nemesis/invoke! (membership-nemesis)
                                    test-config
                                    {:type :info
                                     :f :shrink})]
        (is (= [[:change-membership
                 "n1:21001"
                 ["n1" "n2" "n3" "n4"]]
                [:stop-and-wipe :database "n5"]]
               @calls))
        (is (= {:change :shrink
                :node "n5"
                :before before
                :after after}
               (select-keys (:value result)
                            [:change :node :before :after])))))))

(deftest defers-cleanup-after-an-indeterminate-shrink
  (let [calls (atom [])
        before (set nodes)
        after (disj before "n5")
        statuses (atom [(stable-status before)
                        (stable-status after)])
        subject (membership-nemesis)]
    (with-redefs [cluster/membership-status
                  (fn [_test]
                    (let [status (first @statuses)]
                      (swap! statuses subvec 1)
                      status))
                  random/shuffle reverse
                  client/change-membership!
                  (fn [& _]
                    (swap! calls conj :change-membership)
                    (throw (ex-info "timeout"
                                    {:kind :request-timeout})))
                  openraft-db/stop-and-wipe-node!
                  (fn [_database _test node]
                    (swap! calls conj [:stop-and-wipe node]))]
      (let [pending (nemesis/invoke! subject
                                     test-config
                                     {:type :info
                                      :f :shrink})]
        (is (= :indeterminate (get-in pending [:value :status])))
        (is (= [:change-membership] @calls)))

      (let [resolved (nemesis/invoke! subject
                                      test-config
                                      {:type :info
                                       :f :grow})]
        (is (= [:change-membership [:stop-and-wipe "n5"]] @calls))
        (is (= {:change :shrink
                :before before
                :after after}
               (select-keys (:value resolved)
                            [:change :before :after])))))))

(deftest advances-a-joint-membership-without-waiting
  (let [before #{"n1" "n2" "n3"}
        target (conj before "n4")
        joint {:leader "n1"
               :stable? false
               :effective-voter-configs [before target]}
        calls (atom [])]
    (with-redefs [cluster/membership-status (constantly joint)
                  cluster/await-stable-membership!
                  (fn [& _]
                    (throw (ex-info "runtime joint completion must not wait"
                                    {})))
                  client/change-membership!
                  (fn [endpoint node-ids]
                    (swap! calls conj
                           [:change-membership endpoint node-ids]))]
      (let [result (nemesis/invoke! (membership-nemesis)
                                    test-config
                                    {:type :info
                                     :f :grow})]
        (is (= [[:change-membership
                 "n1:21001"
                 ["n1" "n2" "n3" "n4"]]]
               @calls))
        (is (= {:status :completed
                :target target}
               (select-keys (:value result) [:status :target])))))))

(deftest final-recovery-resolves-a-pending-removal
  (let [before (set nodes)
        after (disj before "n5")
        restored {:leader "n1" :voters before}
        statuses (atom [(stable-status before)
                        (stable-status after)
                        restored])
        pending (atom {:change :shrink
                       :node "n5"
                       :before before
                       :target after})
        subject (membership/->MembershipNemesis :database pending)
        calls (atom [])]
    (with-redefs-fn
      {#'membership/stable-membership!
       (fn [_test]
         (let [status (first @statuses)]
           (swap! statuses subvec 1)
           status))
       #'membership/change-membership-and-await!
       (fn [_test _leader-endpoint target]
         (swap! calls conj [:complete-removal target])
         (stable-status after))
       #'membership/grow!
       (fn [_database _test]
         (swap! calls conj :grow))
       #'openraft-db/stop-and-wipe-node!
       (fn [_database _test node]
         (swap! calls conj [:stop-and-wipe node]))
       #'cluster/await-ready!
       (fn [_test]
         (swap! calls conj :await-ready)
         {:leader "n2"})}
      (fn []
        (let [result (nemesis/invoke! subject
                                      test-config
                                      {:type :info
                                       :f :restore-membership})]
          (is (= [[:complete-removal after]
                  [:stop-and-wipe "n5"]
                  :grow
                  :await-ready]
                 @calls))
          (is (= {:leader "n2"
                  :voters before
                  :resolved-change {:change :shrink
                                    :node "n5"
                                    :before before
                                    :leader "n1"
                                    :after after}}
                 (:value result)))
          (is (nil? @pending)))))))

(deftest final-recovery-confirms-a-completed-grow
  (let [before #{"n1" "n2" "n3" "n4"}
        after (conj before "n5")
        pending (atom {:change :grow
                       :node "n5"
                       :source :non-member
                       :before before
                       :target after})
        subject (membership/->MembershipNemesis :database pending)]
    (with-redefs-fn
      {#'membership/stable-membership! (fn [_test]
                                         (stable-status after))
       #'membership/grow! (fn [& _]
                            (throw (ex-info "MUST NOT grow again" {})))
       #'cluster/await-ready! (fn [_test]
                                {:leader "n1"})}
      (fn []
        (let [result (nemesis/invoke! subject
                                      test-config
                                      {:type :info
                                       :f :restore-membership})]
          (is (= {:change :grow
                  :node "n5"
                  :source :non-member
                  :before before
                  :leader "n1"
                  :after after}
                 (get-in result [:value :resolved-change])))
          (is (nil? @pending)))))))

(deftest preserves-the-minimum-membership
  (let [changed? (atom false)
        status (stable-status #{"n1" "n2" "n3"})]
    (with-redefs [cluster/membership-status (constantly status)
                  client/change-membership!
                  (fn [& _]
                    (reset! changed? true))]
      (let [result (nemesis/invoke! (membership-nemesis)
                                    test-config
                                    {:type :info
                                     :f :shrink})]
        (is (= :minimum-membership (:value result)))
        (is (false? @changed?))))))

(deftest membership-package-requires-room-to-change
  (let [three-node-test (assoc test-config
                               :nodes ["n1" "n2" "n3"])
        error (try
                (membership/membership-package :database three-node-test)
                nil
                (catch clojure.lang.ExceptionInfo e
                  e))]
    (is (= "Membership nemesis requires more than three nodes"
           (ex-message error)))
    (is (= ["n1" "n2" "n3"]
           (:nodes (ex-data error))))))

(deftest checks-membership-coverage-and-restoration
  (let [subject (:checker (membership/membership-package
                           :database
                           test-config))
        all-voters (set nodes)
        four-voters (disj all-voters "n5")
        shrink {:type :info
                :f :grow
                :value {:change :shrink
                        :before all-voters
                        :after four-voters}}
        grow {:type :info
              :f :grow
              :value {:change :grow
                      :before four-voters
                      :after all-voters}}
        restore {:type :info
                 :f :restore-membership
                 :value {:leader "n1"
                         :voters all-voters}}]
    (testing "a resolved change is attributed to its actual fault class"
      (is (:valid? (checker/check subject
                                  test-config
                                  [shrink grow restore]
                                  {}))))

    (testing "an indeterminate request does not count as coverage"
      (let [result (checker/check
                    subject
                    test-config
                    [(assoc shrink
                            :value {:change :shrink
                                    :status :indeterminate
                                    :before all-voters
                                    :target four-voters})
                     grow
                     restore]
                    {})]
        (is (false? (:valid? result)))
        (is (= [:shrink] (:missing-changes result)))))

    (testing "final recovery can confirm an indeterminate change"
      (let [result (checker/check
                    subject
                    test-config
                    [(assoc shrink
                            :value {:change :shrink
                                    :status :indeterminate
                                    :before all-voters
                                    :target four-voters})
                     grow
                     (assoc-in restore
                               [:value :resolved-change]
                               (:value shrink))]
                    {})]
        (is (:valid? result))))

    (testing "the final membership must be restored"
      (let [result (checker/check subject
                                  test-config
                                  [shrink grow]
                                  {})]
        (is (false? (:valid? result)))
        (is (false? (:restored? result)))))))
